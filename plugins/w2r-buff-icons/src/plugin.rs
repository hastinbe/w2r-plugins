//! The game-facing half (32-bit only). Three detours, none of which changes game state:
//!
//! - `UnitBuffIcons` (0x4efa30) and `DrawBuffIcon` (0x4efe50): the game draws a unit's spell
//!   icons at fixed spots 16 px apart (Bloodlust, Haste or Slow, Invisibility, Unholy Armor),
//!   leaving gaps. The detours move the point each icon is drawn from, so the icons are packed from
//!   the left and wrap by unit size (`model::icon_offset`).
//! - `DrawUnitHealthBar` (0x4efb00), called once per drawn unit: with See-Through Health Bars on it
//!   draws the same health and mana rows itself over a see-through track instead of the game's
//!   solid black, and it collects the countdown numbers for the unit's icons.
//!
//! What is collected is drawn by the `w2r_map_draw` export, which the loader calls once a frame in a
//! window over the map view, under the HUD. On a loader without it, the game keeps drawing its own
//! bars and there are no numbers; the icon packing works either way.
//!
//! Nothing goes to w2r.log: the plugin never calls `host.log`. With Debug Log on (plugin manager
//! card only, off by default) it writes its own `debug.log` in its data folder; the two runs before
//! are kept as debug.1.log and debug.2.log. A panic is caught before it can unwind into the game
//! (detours are outside the host's crash guard, docs/abi.md).

use core::ffi::c_void;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use w2r_abi::*;

use crate::model;

/// The rows go on the game's own Gameplay page: See-Through Health Bars under the Health Bars
/// heading, the icon settings under a heading of their own after it.
const CATEGORY: &str = "Gameplay";
const SUBCATEGORY_HEALTH: &str = "Health Bars";
const SUBCATEGORY_ICONS: &str = "Spell Icons";

const KEY_SEE_THROUGH: &str = "see_through_health_bars";
const KEY_FOLLOW: &str = "icons_follow_health_bars";
const KEY_COUNTDOWN: &str = "countdown_numbers";
const KEY_DEBUG: &str = "debug_log";

const LABEL_SEE_THROUGH: &str = "See-Through Health Bars";
const LABEL_FOLLOW: &str = "Buff Icons Follow Health Bars";
const LABEL_COUNTDOWN: &str = "Countdown on Buff Icons";

/// Unit record: position in world pixels, type and owner.
const UNIT_X: usize = 0x00;
const UNIT_Y: usize = 0x02;
const UNIT_TYPE: usize = 0x27;
const UNIT_OWNER: usize = 0x2c;
const MAX_PLAYERS: usize = 8;
/// Unit types the type tables cover.
const TYPE_COUNT: usize = 0x100;
/// Lines written to debug.log per run, at most.
const DEBUG_CAP: u32 = 4000;

/// The countdown, in the game's own font: white at 35 %, no outline, so the icon shows through.
const NUMBER_COLOUR: u32 = 0x59ff_ffff;

struct Game {
    type_flags: *const u32,
    select_box: *const u16,
    draw_centre: *const i16,
    settings_flags: *const u32,
    view_player: *const u8,
    vision: *const u8,
    max_hp: *const u16,
    palette: *const *const u8,
}

// SAFETY: addresses of the game's globals, fixed for the session; used from the game's main thread.
unsafe impl Send for Game {}
unsafe impl Sync for Game {}

static GAME: OnceLock<Game> = OnceLock::new();
static HOST: AtomicPtr<W2rHost> = AtomicPtr::new(core::ptr::null_mut());

/// Settings, read at load, at match start, when the options screen changes them and from the card.
static SEE_THROUGH: AtomicBool = AtomicBool::new(true);
static FOLLOW: AtomicBool = AtomicBool::new(true);
static COUNTDOWN: AtomicBool = AtomicBool::new(true);
static DEBUG: AtomicBool = AtomicBool::new(false);

fn host() -> Option<&'static W2rHost> {
    unsafe { HOST.load(Ordering::Acquire).as_ref() }
}

fn s(text: &str) -> W2rStr {
    W2rStr {
        ptr: text.as_ptr().cast(),
        len: text.len() as u32,
    }
}

/// The selection box (the game's table values, one less than the pixels) of a unit type.
unsafe fn select_box(game: &Game, ty: usize) -> (i32, i32) {
    unsafe { (*game.select_box.add(ty * 2) as i32, *game.select_box.add(ty * 2 + 1) as i32) }
}

/// With health bars below the unit and Buff Icons Follow Health Bars on, the icons start at the
/// bottom of the unit and stack up.
fn icons_from_bottom(game: &Game) -> bool {
    FOLLOW.load(Ordering::Relaxed) && unsafe { *game.settings_flags } & model::FLAG_BELOW != 0
}

// ---- spell icons --------------------------------------------------------------------------------

type BuffIconsFn = unsafe extern "C" fn(*mut u8, i32);
type BuffIconFn = unsafe extern "C" fn(i32, i32, i32, i32, i32, i32);

static BUFF_ICONS_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BUFF_ICONS_HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);
static BUFF_ICON_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BUFF_ICON_HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);
/// Both icon hooks are in, so the icons are packed and the numbers follow the packed spots.
static ICONS_PACKED: AtomicBool = AtomicBool::new(false);
/// The unit whose icons are being drawn, and how many it has drawn so far (main thread only).
static BUFF_UNIT: AtomicUsize = AtomicUsize::new(0);
static BUFF_INDEX: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" fn buff_icons_detour(unit: *mut u8, hidden: i32) {
    let original = BUFF_ICONS_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    let original: BuffIconsFn = unsafe { core::mem::transmute(original) };
    BUFF_UNIT.store(unit as usize, Ordering::Relaxed);
    BUFF_INDEX.store(0, Ordering::Relaxed);
    unsafe { original(unit, hidden) };
    BUFF_UNIT.store(0, Ordering::Relaxed);
}

/// `x`, `y` is the base point (the selection box's top-left, one pixel out); the game adds the art
/// slot's fixed offset. Called for every spell in order, drawing only those whose timer is not 0.
unsafe extern "C" fn buff_icon_detour(x: i32, y: i32, timer: i32, slot: i32, negative_slot: i32, hidden: i32) {
    let original = BUFF_ICON_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    let original: BuffIconFn = unsafe { core::mem::transmute(original) };
    let unit = BUFF_UNIT.load(Ordering::Relaxed);
    let art = if timer > 0 { slot } else { negative_slot };
    // Not drawn (timer 0, hidden), or not from `UnitBuffIcons`: as the game has it.
    if unit == 0 || timer == 0 || hidden & 0xff != 0 || !(0..5).contains(&art) {
        unsafe { original(x, y, timer, slot, negative_slot, hidden) };
        return;
    }
    let index = BUFF_INDEX.fetch_add(1, Ordering::Relaxed);
    let moved = catch_unwind(AssertUnwindSafe(|| {
        let game = GAME.get()?;
        let ty = unsafe { *(unit as *const u8).add(UNIT_TYPE) } as usize;
        if ty >= TYPE_COUNT {
            return None;
        }
        let (w, h) = unsafe { select_box(game, ty) };
        let (dx, dy) = model::icon_offset(index, model::icons_per_row(w), h, icons_from_bottom(game));
        Some((x + dx - model::STOCK_ICON_X[art as usize], y + dy))
    }))
    .ok()
    .flatten();
    let (x, y) = moved.unwrap_or((x, y));
    unsafe { original(x, y, timer, slot, negative_slot, hidden) };
}

// ---- collecting, then drawing in w2r_map_draw ---------------------------------------------------

/// The loader's map-draw table, mirrored here because the pinned ABI crate predates it.
#[repr(C)]
pub struct MapDraw {
    struct_size: u32,
    ctx: *mut c_void,
    origin_x: i32,
    origin_y: i32,
    view_x: f32,
    view_y: f32,
    view_w: f32,
    view_h: f32,
    fill_rect: unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, u32),
    text: unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, W2rStr, u32),
}

/// In world pixels: one of the game's health or mana bars, or one countdown number on an icon.
#[derive(Clone, Copy)]
enum Item {
    Bar { x: i32, y: i32, w: i32, fill: f32, colour: u32 },
    Number { x: i32, y: i32, secs: i32 },
}

struct Collected {
    frame: u64,
    items: Vec<Item>,
}

static COLLECTED: Mutex<Collected> = Mutex::new(Collected { frame: 0, items: Vec::new() });
/// Counts `w2r_map_draw` calls; the unit drawing starts a new collection when it moves on. 0 until
/// the loader first calls it: until then the game keeps drawing its own health bars.
static FRAME: AtomicU64 = AtomicU64::new(0);

fn collect(item: Item) {
    let now = FRAME.load(Ordering::Relaxed);
    let mut c = COLLECTED.lock().unwrap_or_else(|e| e.into_inner());
    if c.frame != now {
        c.frame = now;
        c.items.clear();
    }
    if c.items.len() < 4096 {
        c.items.push(item);
    }
}

/// Called by the loader every frame of a match or replay, inside a window over the map view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn w2r_map_draw(draw: *const MapDraw) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let Some(d) = draw.as_ref() else { return };
        if (d.struct_size as usize) < core::mem::size_of::<MapDraw>() {
            return;
        }
        let now = FRAME.fetch_add(1, Ordering::Relaxed) + 1;
        let items = {
            let c = COLLECTED.lock().unwrap_or_else(|e| e.into_inner());
            // Collected this frame or the one before (the units may be drawn after this call);
            // anything older is gone from the screen.
            if c.frame + 1 < now { Vec::new() } else { c.items.clone() }
        };
        let (ox, oy) = (d.origin_x as f32, d.origin_y as f32);
        for item in &items {
            match *item {
                Item::Bar { x, y, w, fill, colour } => {
                    let (sx, sy) = (ox + x as f32, oy + y as f32);
                    (d.fill_rect)(d.ctx, sx, sy, w as f32, model::ROW as f32, model::TRACK);
                    let fw = model::stock_filled(w, fill);
                    if fw > 0 {
                        (d.fill_rect)(d.ctx, sx, sy, fw as f32, model::ROW as f32, colour);
                    }
                }
                Item::Number { x, y, secs } => {
                    // Centred on the icon, in a box twice its width so "10" is not cut off.
                    let text = secs.to_string();
                    let sz = model::ICON as f32;
                    (d.text)(d.ctx, ox + x as f32 - sz / 2.0, oy + y as f32, sz * 2.0, sz, s(&text), NUMBER_COLOUR);
                }
            }
        }
    }));
}

/// Whether the viewer may see this owner's countdowns: the viewed player's own units, and those of
/// players whose vision the viewed player shares. Enemies never get them.
fn visible_owner(game: &Game, owner: u8) -> bool {
    let view = unsafe { *game.view_player };
    if owner == view {
        return true;
    }
    if owner as usize >= MAX_PLAYERS || view as usize >= MAX_PLAYERS {
        return false;
    }
    let mask = unsafe { *game.vision.add(view as usize) };
    mask & (1 << owner) != 0
}

/// A palette entry's colour, packed `0xAABBGGRR` opaque.
unsafe fn palette_colour(game: &Game, entry: u8) -> u32 {
    unsafe {
        let pal = *game.palette;
        if pal.is_null() {
            return 0xff00_0000;
        }
        let p = pal.add(entry as usize * 4);
        (*p as u32) | ((*p.add(1) as u32) << 8) | ((*p.add(2) as u32) << 16) | 0xff00_0000
    }
}

/// Things already written to debug.log once this match.
static LOGGED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn debug_once(key: String, msg: impl FnOnce() -> String) {
    if !DEBUG.load(Ordering::Relaxed) {
        return;
    }
    let first = LOGGED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(key);
    if first {
        debug(&msg());
    }
}

/// The health and mana bars (with See-Through Health Bars on) and the countdown numbers of one unit.
unsafe fn collect_unit(game: &Game, unit: *const u8, see_through: bool) {
    unsafe {
        let rec = core::slice::from_raw_parts(unit, model::RECORD);
        let ty = rec[UNIT_TYPE] as usize;
        if ty >= TYPE_COUNT {
            return;
        }
        let flags = *game.type_flags.add(ty);
        let (w, h) = select_box(game, ty);
        let cx = *game.draw_centre.add(ty * 2) as i32;
        let cy = *game.draw_centre.add(ty * 2 + 1) as i32;
        let ux = i16::from_le_bytes([rec[UNIT_X], rec[UNIT_X + 1]]) as i32;
        let uy = i16::from_le_bytes([rec[UNIT_Y], rec[UNIT_Y + 1]]) as i32;
        // The stock bar's left edge (also the icons' base x) and the selection box's top.
        let x = cx - w / 2 + ux - 1;
        let top = cy - h / 2 + uy;
        if see_through {
            let caster = flags & model::TYPE_CASTER != 0;
            let rows = model::stock_rows(top, h, *game.settings_flags, caster, model::hp(rec), *game.max_hp.add(ty), model::mana(rec));
            for row in rows {
                let colour = palette_colour(game, row.palette);
                collect(Item::Bar { x, y: row.y, w: w + 1, fill: row.fill, colour });
            }
        }
        if !COUNTDOWN.load(Ordering::Relaxed) || flags & model::TYPE_BUILDING != 0 || !visible_owner(game, rec[UNIT_OWNER]) {
            return;
        }
        let numbers = model::icon_numbers(rec);
        if numbers.is_empty() {
            return;
        }
        // Where each icon is: packed and wrapped by the icon hooks, or the game's fixed spots when
        // they are not in.
        let arts = model::icon_arts(rec);
        let per_row = model::icons_per_row(w);
        let bottom = icons_from_bottom(game);
        let packed = ICONS_PACKED.load(Ordering::Relaxed);
        for (index, secs) in numbers {
            let (dx, dy) = if packed {
                model::icon_offset(index, per_row, h, bottom)
            } else {
                let art = arts.get(index as usize).copied().unwrap_or(0).min(4);
                (model::STOCK_ICON_X[art], 0)
            };
            let (nx, ny) = (x + dx, top - 1 + dy);
            debug_once(format!("num:{ty}:{index}"), || {
                format!("countdown {secs} s on icon {index} of type {ty} at world ({nx},{ny}), {per_row} per row, bottom={bottom}")
            });
            collect(Item::Number { x: nx, y: ny, secs });
        }
    }
}

// ---- health bars ---------------------------------------------------------------------------------

type HealthBarFn = unsafe extern "C" fn(*mut u8, u32);

static HEALTH_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static HEALTH_HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);

unsafe extern "C" fn health_bar_detour(unit: *mut u8, skip: u32) {
    // See-through: the same rows are drawn in w2r_map_draw instead, once the loader calls it.
    let see_through = SEE_THROUGH.load(Ordering::Relaxed) && GAME.get().is_some() && FRAME.load(Ordering::Relaxed) > 0;
    if !see_through {
        let original = HEALTH_ORIGINAL.load(Ordering::Acquire);
        if original != 0 {
            let original: HealthBarFn = unsafe { core::mem::transmute(original) };
            unsafe { original(unit, skip) };
        }
    }
    // The second pass (skip set) draws nothing in the original either.
    if skip & 0xff != 0 || unit.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(game) = GAME.get() {
            unsafe { collect_unit(game, unit, see_through) };
        }
    }));
}

// ---- debug.log ---------------------------------------------------------------------------------

static DEBUG_FILE: Mutex<Option<W2rHandle>> = Mutex::new(None);
static DEBUG_LINES: AtomicU32 = AtomicU32::new(0);

fn read_all(h: &W2rHost, name: &str) -> Option<Vec<u8>> {
    let mut f = 0;
    if unsafe { (h.data_open)(h.token, s(name), W2R_FILE_READ, &mut f) } != W2R_OK {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let mut got = 0u32;
        let st = unsafe { (h.data_read)(h.token, f, buf.as_mut_ptr().cast(), buf.len() as u32, &mut got) };
        if st != W2R_OK || got == 0 {
            break;
        }
        out.extend_from_slice(&buf[..got as usize]);
        if out.len() > (4 << 20) {
            break;
        }
    }
    unsafe { (h.data_close)(h.token, f) };
    Some(out)
}

fn write_all(h: &W2rHost, name: &str, bytes: &[u8]) {
    let mut f = 0;
    if unsafe { (h.data_open)(h.token, s(name), W2R_FILE_WRITE, &mut f) } == W2R_OK {
        unsafe {
            (h.data_write)(h.token, f, bytes.as_ptr().cast(), bytes.len() as u32);
            (h.data_close)(h.token, f);
        }
    }
}

/// Keeps the last two runs (debug.log, then debug.1.log, then debug.2.log) and opens a new debug.log.
fn open_debug_log() {
    let Some(h) = host() else { return };
    if let Some(old) = read_all(h, "debug.1.log") {
        write_all(h, "debug.2.log", &old);
    }
    if let Some(old) = read_all(h, "debug.log") {
        write_all(h, "debug.1.log", &old);
    }
    let mut f = 0;
    if unsafe { (h.data_open)(h.token, s("debug.log"), W2R_FILE_WRITE, &mut f) } == W2R_OK {
        *DEBUG_FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }
}

fn close_debug_log() {
    let f = DEBUG_FILE.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let (Some(h), Some(f)) = (host(), f) {
        unsafe { (h.data_close)(h.token, f) };
    }
}

fn debug(msg: &str) {
    if !DEBUG.load(Ordering::Relaxed) {
        return;
    }
    let n = DEBUG_LINES.fetch_add(1, Ordering::Relaxed);
    if n > DEBUG_CAP {
        return;
    }
    let line = if n == DEBUG_CAP { "debug.log line limit reached; nothing more this run\n".to_string() } else { format!("{msg}\n") };
    let guard = DEBUG_FILE.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(h), Some(f)) = (host(), *guard) {
        unsafe { (h.data_write)(h.token, f, line.as_ptr().cast(), line.len() as u32) };
    }
}

/// Turns Debug Log on or off, saves it, and opens or closes debug.log.
fn set_debug(on: bool) {
    if let Some(h) = host() {
        unsafe { (h.settings_set_i64)(h.token, s(KEY_DEBUG), on as i64) };
    }
    if on == DEBUG.swap(on, Ordering::Relaxed) {
        return;
    }
    if on {
        open_debug_log();
        debug(&format!("Debug Log turned on (Buff Icons {})", env!("CARGO_PKG_VERSION")));
    } else {
        debug("Debug Log turned off");
        close_debug_log();
    }
}

// ---- symbols and settings -------------------------------------------------------------------

fn handle_of(name: &str) -> Option<W2rHandle> {
    let h = host()?;
    let mut handle = 0;
    (unsafe { (h.symbol_lookup)(h.token, s(name), &mut handle) } == W2R_OK).then_some(handle)
}

fn lookup(name: &str) -> Option<usize> {
    let h = host()?;
    let handle = handle_of(name)?;
    let mut addr = 0usize;
    (unsafe { (h.symbol_address)(h.token, handle, &mut addr) } == W2R_OK).then_some(addr)
}

fn resolve() -> Result<Game, &'static str> {
    let at = |name: &'static str| lookup(name).ok_or(name);
    Ok(Game {
        type_flags: at("UnitTypeFlags")? as *const u32,
        select_box: at("UnitSelectBox")? as *const u16,
        draw_centre: at("UnitDrawCentre")? as *const i16,
        settings_flags: at("GameSettingsFlags")? as *const u32,
        view_player: at("ViewPlayer")? as *const u8,
        vision: at("VisionMasks")? as *const u8,
        max_hp: at("UnitMaxHp")? as *const u16,
        palette: at("RenderPalette")? as *const *const u8,
    })
}

fn setting(key: &str, default: i64) -> i64 {
    let mut v = 0i64;
    match host() {
        Some(h) if unsafe { (h.settings_get_i64)(h.token, s(key), &mut v) } == W2R_OK => v,
        _ => default,
    }
}

fn read_settings() {
    SEE_THROUGH.store(setting(KEY_SEE_THROUGH, 1) != 0, Ordering::Relaxed);
    FOLLOW.store(setting(KEY_FOLLOW, 1) != 0, Ordering::Relaxed);
    COUNTDOWN.store(setting(KEY_COUNTDOWN, 1) != 0, Ordering::Relaxed);
    DEBUG.store(setting(KEY_DEBUG, 0) != 0, Ordering::Relaxed);
}

fn def(key: &str, label: &str, description: &str, subcategory: &str) -> W2rSettingDef {
    W2rSettingDef {
        struct_size: W2R_SETTING_DEF_V1_SIZE,
        kind: W2R_SETTING_BOOL,
        default_int: 1,
        min: 0,
        max: 0,
        flags: 0,
        choice_count: 0,
        choices: core::ptr::null(),
        reserved: 0,
        key: s(key),
        label: s(label),
        description: s(description),
        category: s(CATEGORY),
        subcategory: s(subcategory),
        default_str: s(""),
        active_when: s(""),
    }
}

/// Declares the options-screen rows; false on an older host (the defaults still apply).
fn declare() -> bool {
    let Some(h) = host() else { return false };
    if h.struct_size < W2R_HOST_SETTINGS_DECLARE_SIZE {
        return false;
    }
    let rows = [
        def(
            KEY_SEE_THROUGH,
            LABEL_SEE_THROUGH,
            "The health and mana bars over units get a see-through black instead of solid black. Off: the game draws them as before.",
            SUBCATEGORY_HEALTH,
        ),
        def(
            KEY_FOLLOW,
            LABEL_FOLLOW,
            "With health bars below units, the spell icons start at the bottom of the unit and stack up. Off: they stay at the top, as the game draws them.",
            SUBCATEGORY_ICONS,
        ),
        def(
            KEY_COUNTDOWN,
            LABEL_COUNTDOWN,
            "The seconds left on a spell icon in its last 10 seconds, on your units and your allies'.",
            SUBCATEGORY_ICONS,
        ),
    ];
    rows.iter().fold(true, |ok, d| ok & (unsafe { (h.settings_declare)(h.token, d) } == W2R_OK))
}

// ---- plugin table --------------------------------------------------------------------------------

unsafe extern "C" fn on_event(event: *const W2rEvent) {
    let Some(event) = (unsafe { event.as_ref() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| match event.id {
        W2R_EVT_MATCH_START => {
            read_settings();
            *LOGGED.lock().unwrap_or_else(|e| e.into_inner()) = None;
            debug(&format!(
                "match start: see-through {}, follow {}, countdown {}, icons packed {}, map drawing {}",
                SEE_THROUGH.load(Ordering::Relaxed),
                FOLLOW.load(Ordering::Relaxed),
                COUNTDOWN.load(Ordering::Relaxed),
                ICONS_PACKED.load(Ordering::Relaxed),
                FRAME.load(Ordering::Relaxed) > 0
            ));
        }
        W2R_EVT_SETTINGS_CHANGED => read_settings(),
        _ => {}
    }));
}

/// A setting as a checkbox on the plugin card; a change is saved and applied at once.
fn card_checkbox(ui: &W2rUi, key: &str, label: &str) {
    let mut on = (setting(key, 1) != 0) as i32;
    if unsafe { (ui.checkbox)(ui.ctx, s(label), &mut on) } != 0 {
        if let Some(h) = host() {
            unsafe { (h.settings_set_i64)(h.token, s(key), (on != 0) as i64) };
        }
        read_settings();
    }
}

unsafe extern "C" fn on_ui(ui: *const W2rUi) {
    let Some(ui) = (unsafe { ui.as_ref() }) else {
        return;
    };
    unsafe {
        (ui.label)(ui.ctx, s("Spell icons over units, packed side by side and wrapped by unit size."));
        (ui.label)(ui.ctx, s("Also under Options > Gameplay (Health Bars and Spell Icons)."));
        // The same settings as the options screen, in the same order and under the same keys.
        (ui.label)(ui.ctx, s(SUBCATEGORY_HEALTH));
        card_checkbox(ui, KEY_SEE_THROUGH, LABEL_SEE_THROUGH);
        (ui.label)(ui.ctx, s(SUBCATEGORY_ICONS));
        card_checkbox(ui, KEY_FOLLOW, LABEL_FOLLOW);
        card_checkbox(ui, KEY_COUNTDOWN, LABEL_COUNTDOWN);
        // Debug Log is here only, not on the options screen.
        (ui.label)(ui.ctx, s("Testing"));
        let mut on = DEBUG.load(Ordering::Relaxed) as i32;
        if (ui.checkbox)(ui.ctx, s("Debug Log (debug.log in the plugin's data folder)"), &mut on) != 0 {
            set_debug(on != 0);
        }
    }
}

/// Installs one hook; its trampoline goes to `original` and its handle to `slot`.
fn hook(h: &W2rHost, name: &str, detour: *mut c_void, original: &AtomicUsize, slot: &Mutex<Option<W2rHandle>>) -> bool {
    let Some(target) = handle_of(name) else {
        debug(&format!("symbol {name} not available on this loader"));
        return false;
    };
    let mut trampoline: *mut c_void = core::ptr::null_mut();
    let mut handle = 0;
    let status = unsafe { (h.hook_install)(h.token, target, detour, &mut trampoline, &mut handle) };
    if status != W2R_OK {
        debug(&format!("{name}: hook_install returned {status}"));
        return false;
    }
    original.store(trampoline as usize, Ordering::Release);
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    true
}

fn unhook(slot: &Mutex<Option<W2rHandle>>) {
    let handle = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let (Some(h), Some(handle)) = (host(), handle) {
        unsafe { (h.hook_remove)(h.token, handle) };
    }
}

unsafe extern "C" fn on_load(host: *const W2rHost) -> W2rStatus {
    HOST.store(host.cast_mut(), Ordering::Release);
    let declared = declare();
    read_settings();
    if DEBUG.load(Ordering::Relaxed) {
        open_debug_log();
    }
    debug(&format!("Buff Icons {} loading; settings declared: {declared}", env!("CARGO_PKG_VERSION")));
    let game = match resolve() {
        Ok(g) => g,
        Err(name) => {
            debug(&format!("symbol {name} not available on this loader; nothing to do"));
            return W2R_E_STATE;
        }
    };
    let _ = GAME.set(game);
    let Some(h) = self::host() else {
        return W2R_E_STATE;
    };
    for id in [W2R_EVT_MATCH_START, W2R_EVT_SETTINGS_CHANGED] {
        if unsafe { (h.subscribe)(h.token, id) } != W2R_OK {
            debug(&format!("cannot subscribe to event {id}"));
        }
    }
    // The icons: both hooks or neither (then the game places them, and the numbers follow it).
    let packed = hook(h, "DrawBuffIcon", buff_icon_detour as *mut c_void, &BUFF_ICON_ORIGINAL, &BUFF_ICON_HOOK)
        && hook(h, "UnitBuffIcons", buff_icons_detour as *mut c_void, &BUFF_ICONS_ORIGINAL, &BUFF_ICONS_HOOK);
    if !packed {
        unhook(&BUFF_ICON_HOOK);
    }
    ICONS_PACKED.store(packed, Ordering::Relaxed);
    let health = hook(h, "DrawUnitHealthBar", health_bar_detour as *mut c_void, &HEALTH_ORIGINAL, &HEALTH_HOOK);
    debug(&format!("icons packed: {packed}; health bar hook: {health}"));
    if packed || health { W2R_OK } else { W2R_E_STATE }
}

unsafe extern "C" fn on_unload() {
    ICONS_PACKED.store(false, Ordering::Relaxed);
    for slot in [&HEALTH_HOOK, &BUFF_ICONS_HOOK, &BUFF_ICON_HOOK] {
        unhook(slot);
    }
    debug("unloading");
    close_debug_log();
}

static TABLE: W2rPlugin = W2rPlugin {
    struct_size: W2R_PLUGIN_V1_SIZE,
    on_load: Some(on_load),
    on_event: Some(on_event),
    on_ui: Some(on_ui),
    on_unload: Some(on_unload),
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn w2r_plugin_main(_host: *const W2rHost) -> *const W2rPlugin {
    &TABLE
}
