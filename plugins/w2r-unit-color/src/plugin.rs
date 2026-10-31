//! The game-facing half (32-bit only). One detour on `HudMenuButtons` (the HUD menu buttons, drawn
//! every frame in a match; HudFoodCell, HudResourceCell and HudBeginFrame belong to other plugins
//! and a function takes one hook) is the per-frame tick: it calls the original, then makes the
//! eight entries of the HD colour table and the seven team ramps of the render palette match the
//! settings. Panics are caught before they unwind into the game (detours are outside the host's
//! crash guard, docs/abi.md).
//!
//! On a host with declared settings, the eight colours are colour pickers on the options screen
//! (Unit Color > Player Colors). The plugin's card in the plugin manager picks, saves and reloads
//! the profiles in `color_profiles.json`, which lives in the plugin's own data folder, the only
//! folder a plugin can write (docs/abi.md, `data_open`). On an older host the card also has one
//! text box per colour.
//!
//! Allied Minimap Colors (after Lambchops' Allied Colors for the Battle.net
//! Edition): the same tick rewrites the game's eight-byte per-player minimap colour table, allies
//! blue and enemies red, and polls the toggle key. The table has no host symbol, so it is found at
//! a fixed distance from `TeamColourRgba` for build 1.0.2.2818 and used only after its stock copy
//! is seen holding the expected bytes (see `setup_minimap`).

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use w2r_abi::*;

use crate::allied;
use crate::model::{self, Colors, PLAYERS, Profile, RAMP, Rgb};
use crate::profiles;

/// Bytes per render palette entry (RGBA).
const PALETTE_STRIDE: usize = 4;
/// Floats per HD colour table entry (RGBA).
const TABLE_STRIDE: usize = 4;

/// The profiles file in the plugin's data folder.
const PROFILES_FILE: &str = "color_profiles.json";
/// Largest file read (64 full profiles are about 20 KiB).
const MAX_FILE: usize = 1 << 20;
/// Edit buffers: `#RRGGBB` plus room for stray spaces; a profile name.
const HEX_CAP: usize = 16;
const NAME_CAP: usize = model::MAX_NAME + 1;

/// Where the colour pickers appear on the options screen.
const CATEGORY: &str = "Unit Color";
/// The heading over the profile dropdown and the save and delete rows (the screen draws one over
/// every group of rows, so it cannot be left out). At most 32 bytes.
const PROFILE_SUBCATEGORY: &str = "Profiles";
// The host refuses a category or subcategory over 32 bytes, and with it the whole declaration.
const _: () = assert!(
    CATEGORY.len() <= 32
        && PROFILE_SUBCATEGORY.len() <= 32
        && SUBCATEGORY.len() <= 32
        && ALLIED_SUBCATEGORY.len() <= 32
);
const SUBCATEGORY: &str = "Player Colors";
const ALLIED_SUBCATEGORY: &str = "Allied Minimap Colors";

/// Build 1.0.2.2818: `TeamColourRgba` (0x008c9640) minus the live minimap colour table
/// (0x008c8d64, read by the minimap dot drawing at 0x004c2d6b and 0x004f8531). The stock copy by
/// colour index follows 0x20 bytes later (0x008c8d84; the match start copies it into the live one
/// at 0x004f09c3). Found in the game's code, not a host symbol.
const MINIMAP_BEFORE_TABLE: usize = 0x8dc;
const MINIMAP_STOCK_AFTER_LIVE: usize = 0x20;

type MenuButtonsFn = unsafe extern "C" fn(i32);

/// `W2rUi.color_edit`: a caption, a swatch and a button that opens the host's color editor over
/// a 7-byte `rrggbb` + NUL buffer; 1 when OK picked a different color. Appended to `W2rUi` by a
/// W2R Mod Loader with card widgets (hastinbe/w2r-mod-loader#5); the w2r-abi this
/// workspace pins predates it, so the fields are declared here and used only when
/// `struct_size` says the host has them.
type ColorEditFn = unsafe extern "C" fn(*mut c_void, W2rStr, *mut u8) -> i32;
/// `W2rUi.combo`, appended after `color_edit` by the same loader: a caption and a dropdown of
/// `count` choices (2 to 16, at most 64 bytes each) showing `*selected`; 1 when the player picked
/// a different entry.
type ComboFn = unsafe extern "C" fn(*mut c_void, W2rStr, *const W2rStr, u32, *mut i32) -> i32;

/// `W2rUi` with the card widget fields appended.
#[repr(C)]
struct UiExt {
    base: W2rUi,
    color_edit: ColorEditFn,
    combo: ComboFn,
}

/// Whether the host's widget table reaches the end of the field at `offset`.
fn ui_has(ui: &W2rUi, offset: usize) -> bool {
    ui.struct_size as usize >= offset + core::mem::size_of::<usize>()
}

/// The host's `color_edit`, when its widget table is long enough to have one.
fn color_edit_of(ui: &W2rUi) -> Option<ColorEditFn> {
    if !ui_has(ui, core::mem::offset_of!(UiExt, color_edit)) {
        return None;
    }
    // SAFETY: the host says its table is at least this long, and the field is appended after the
    // ones `W2rUi` declares.
    Some(unsafe { (*(ui as *const W2rUi).cast::<UiExt>()).color_edit })
}

/// The host's `combo`, when its widget table is long enough to have one.
fn combo_of(ui: &W2rUi) -> Option<ComboFn> {
    if !ui_has(ui, core::mem::offset_of!(UiExt, combo)) {
        return None;
    }
    // SAFETY: as in `color_edit_of`.
    Some(unsafe { (*(ui as *const W2rUi).cast::<UiExt>()).combo })
}

/// What Allied Minimap Colors reads and writes.
struct Minimap {
    live: *mut u8,
    local: *const u8,
    stance: *const u8,
    reorder: *const u8,
    /// Non-zero while the chat box is open (what w2r-chat reads as "composing").
    chat_mode: *const u8,
}

// SAFETY: addresses of the game's globals, fixed for the session; used from the main thread.
unsafe impl Send for Minimap {}
unsafe impl Sync for Minimap {}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(vkey: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

struct Game {
    table: *mut f32,
    palette: *const *mut u8,
}

// SAFETY: addresses of the game's globals, fixed for the session; used from the main thread.
unsafe impl Send for Game {}
unsafe impl Sync for Game {}

/// What we know about one colour's game entries, so the stock values are never lost and a foreign
/// change (the game rebuilding its palette) is noticed.
#[derive(Clone, Copy)]
struct Slot {
    stock_table: Option<[f32; 4]>,
    stock_ramp: [Rgb; RAMP],
    written_ramp: [Rgb; RAMP],
    owns_ramp: bool,
}

const EMPTY_SLOT: Slot = Slot {
    stock_table: None,
    stock_ramp: [[0; 3]; RAMP],
    written_ramp: [[0; 3]; RAMP],
    owns_ramp: false,
};

/// The plugin's card in the plugin manager.
struct Page {
    /// The profiles in `color_profiles.json`, in file order.
    profiles: Vec<Profile>,
    /// False while the file could not be read: saving would overwrite what is in it.
    file_ok: bool,
    /// Slider position: 0 is "Game default", n is `profiles[n - 1]`.
    selected: i32,
    hex: [[u8; HEX_CAP]; PLAYERS],
    name: [u8; NAME_CAP],
    status: String,
    /// The profile the card's Delete button is asking about ("are you sure?"), if any.
    confirm_delete: Option<String>,
}

static GAME: OnceLock<Game> = OnceLock::new();
static HOST: AtomicPtr<W2rHost> = AtomicPtr::new(core::ptr::null_mut());
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);
/// The wanted colours (`None`: the game's own); read by the tick, written by the page and events.
static COLORS: Mutex<Colors> = Mutex::new([None; PLAYERS]);
/// The game's own colours, read from the HD table before anything was written.
static STOCK: Mutex<[Rgb; PLAYERS]> = Mutex::new(model::REMASTERED);
/// Whether the options screen shows the colour pickers (the host took the declarations).
static DECLARED: AtomicBool = AtomicBool::new(false);
/// The profile names in the options screen's dropdown (after Custom and Game default), as they
/// were at load; the host does not let a dropdown's list change later.
static LISTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The dropdown index last stored, to tell a new pick from a colour edit on Apply.
static LAST_CHOICE: Mutex<i64> = Mutex::new(model::CHOICE_GAME_DEFAULT);
static SLOTS: Mutex<[Slot; PLAYERS]> = Mutex::new([EMPTY_SLOT; PLAYERS]);
static PAGE: Mutex<Option<Page>> = Mutex::new(None);
/// Set when the minimap table was found and checked; Allied Minimap Colors is off without it.
static MINIMAP: OnceLock<Minimap> = OnceLock::new();
/// Allied Minimap Colors on (the `allied_minimap` setting).
static ALLIED: AtomicBool = AtomicBool::new(false);
/// The toggle key's virtual-key code.
static HOTKEY: AtomicI32 = AtomicI32::new(0x70);
/// True while the live minimap table holds what the plugin wrote.
static MINIMAP_OWNED: AtomicBool = AtomicBool::new(false);
static KEY: Mutex<allied::Toggle> = Mutex::new(allied::Toggle::new());

fn host() -> Option<&'static W2rHost> {
    unsafe { HOST.load(Ordering::Acquire).as_ref() }
}

fn s(text: &str) -> W2rStr {
    W2rStr {
        ptr: text.as_ptr().cast(),
        len: text.len() as u32,
    }
}

fn log(level: u32, msg: &str) {
    if let Some(h) = host() {
        unsafe { (h.log)(h.token, level, s(msg)) };
    }
}

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

fn colors() -> Colors {
    *COLORS.lock().unwrap_or_else(|e| e.into_inner())
}

fn stock() -> [Rgb; PLAYERS] {
    *STOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- settings --------------------------------------------------------------------------------

/// The value under `key`, or empty when it is missing or unreadable.
fn get_str(key: &str) -> String {
    let Some(h) = host() else {
        return String::new();
    };
    let mut buf = [0u8; 128];
    let mut len = 0u32;
    let st = unsafe {
        (h.settings_get_str)(
            h.token,
            s(key),
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut len,
        )
    };
    if st != W2R_OK {
        return String::new();
    }
    String::from_utf8_lossy(&buf[..(len as usize).min(buf.len())]).into_owned()
}

fn set_str(key: &str, value: &str) {
    let Some(h) = host() else { return };
    let st = unsafe { (h.settings_set_str)(h.token, s(key), s(value)) };
    if st != W2R_OK {
        log(W2R_LOG_WARN, &format!("saving {key}: status {st}"));
    }
}

/// The colours in the settings. A value equal to the game's own colour, or one mangled by hand,
/// means "the game's own".
fn load_colors() -> Colors {
    let stock = stock();
    let mut out = [None; PLAYERS];
    for (i, c) in out.iter_mut().enumerate() {
        *c = model::wanted(&get_str(&model::key(i)), stock[i]);
    }
    out
}

/// Stores colour `i` the way the options screen's picker does (`rrggbb`; the game's own colour
/// is stored as itself, so the picker shows it).
fn save_one(i: usize, c: Option<Rgb>) {
    let value = model::to_setting(c.unwrap_or(stock()[i]));
    if get_str(&model::key(i)) != value {
        set_str(&model::key(i), &value);
    }
}

/// Makes `next` the wanted colours and stores them.
fn set_colors(next: Colors) {
    for (i, c) in next.iter().enumerate() {
        save_one(i, *c);
    }
    *COLORS.lock().unwrap_or_else(|e| e.into_inner()) = next;
}

/// One declaration with every field the kind does not use left empty.
fn def(kind: u32, key: &str, label: &str, description: &str, subcategory: &str) -> W2rSettingDef {
    W2rSettingDef {
        struct_size: W2R_SETTING_DEF_V1_SIZE,
        kind,
        default_int: 0,
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

/// Declares the profile dropdown and the eight colour pickers for the options screen; false on
/// an older host or if the host refused one (it logs which).
fn declare(stock: &[Rgb; PLAYERS], profiles: &[Profile]) -> bool {
    let Some(h) = host() else { return false };
    if h.struct_size < W2R_HOST_SETTINGS_DECLARE_SIZE {
        return false;
    }
    let mut all = true;
    let mut submit = |d: &W2rSettingDef, key: &str| {
        let st = unsafe { (h.settings_declare)(h.token, d) };
        if st != W2R_OK {
            log(W2R_LOG_WARN, &format!("declaring {key}: status {st}"));
            all = false;
        }
    };

    // The dropdown: its labels must outlive the call only, so they live on this stack frame.
    let labels = model::choice_labels(profiles);
    let strs: Vec<W2rStr> = labels.iter().map(|l| s(l)).collect();
    let mut d = def(
        W2R_SETTING_CHOICE,
        model::PROFILE_KEY,
        "Color Profile",
        "Choose a profile and press Apply to use its colors. The list is read when the game starts.",
        PROFILE_SUBCATEGORY,
    );
    d.default_int = model::CHOICE_GAME_DEFAULT;
    d.choices = strs.as_ptr();
    d.choice_count = strs.len() as u32;
    submit(&d, model::PROFILE_KEY);
    *LISTED.lock().unwrap_or_else(|e| e.into_inner()) = model::listed_names(profiles);

    // Saving and deleting from the options screen: the profile's name, a checkbox that makes the
    // next Apply save under it, and two that make it delete it (the second is the "are you sure",
    // greyed until the first is checked). The plugin clears the boxes after.
    let d = def(
        W2R_SETTING_STRING,
        model::NAME_KEY,
        model::NAME_LABEL,
        model::NAME_DESCRIPTION,
        PROFILE_SUBCATEGORY,
    );
    submit(&d, model::NAME_KEY);
    let d = def(
        W2R_SETTING_BOOL,
        model::SAVE_KEY,
        model::SAVE_LABEL,
        model::SAVE_DESCRIPTION,
        PROFILE_SUBCATEGORY,
    );
    submit(&d, model::SAVE_KEY);
    let d = def(
        W2R_SETTING_BOOL,
        model::DELETE_KEY,
        model::DELETE_LABEL,
        model::DELETE_DESCRIPTION,
        PROFILE_SUBCATEGORY,
    );
    submit(&d, model::DELETE_KEY);
    let mut d = def(
        W2R_SETTING_BOOL,
        model::CONFIRM_KEY,
        model::CONFIRM_LABEL,
        model::CONFIRM_DESCRIPTION,
        PROFILE_SUBCATEGORY,
    );
    d.active_when = s(model::DELETE_KEY);
    submit(&d, model::CONFIRM_KEY);

    for (i, rgb) in stock.iter().enumerate() {
        let key = model::key(i);
        let label = format!("Player {} ({})", i + 1, model::NAMES[i]);
        let description = model::color_description(i);
        let default = model::to_setting(*rgb);
        let mut d = def(W2R_SETTING_COLOR, &key, &label, &description, SUBCATEGORY);
        d.default_str = s(&default);
        submit(&d, &key);
    }

    // Allied Minimap Colors, only when the minimap table was found.
    if MINIMAP.get().is_some() {
        let d = def(
            W2R_SETTING_BOOL,
            allied::ENABLED_KEY,
            allied::ENABLED_LABEL,
            allied::ENABLED_DESCRIPTION,
            ALLIED_SUBCATEGORY,
        );
        submit(&d, allied::ENABLED_KEY);
        let keys: Vec<W2rStr> = allied::HOTKEYS.iter().map(|(l, _)| s(l)).collect();
        let mut d = def(
            W2R_SETTING_CHOICE,
            allied::HOTKEY_KEY,
            allied::HOTKEY_LABEL,
            allied::HOTKEY_DESCRIPTION,
            ALLIED_SUBCATEGORY,
        );
        d.default_int = allied::DEFAULT_HOTKEY;
        d.choices = keys.as_ptr();
        d.choice_count = keys.len() as u32;
        submit(&d, allied::HOTKEY_KEY);
    }
    all
}

/// Stores the dropdown index that describes the colours in use, so the options screen shows the
/// right entry the next time it opens.
fn sync_choice(profiles: &[Profile]) {
    if !DECLARED.load(Ordering::Relaxed) {
        return;
    }
    let cur = colors();
    let matched = model::matching(profiles, &cur, &stock()).map(|p| p.name.as_str());
    let listed = LISTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let index = model::choice_for(&listed, cur == [None; PLAYERS], matched);
    let mut last = LAST_CHOICE.lock().unwrap_or_else(|e| e.into_inner());
    if *last != index {
        if let Some(h) = host() {
            unsafe { (h.settings_set_i64)(h.token, s(model::PROFILE_KEY), index) };
        }
        *last = index;
        drop(last);
        follow_name(&listed, index, profiles);
    }
}

/// Makes the options screen's profile name field match dropdown entry `index` (see
/// `model::name_for_choice`). Shown the next time the Unit Color page is opened.
fn follow_name(listed: &[String], index: i64, profiles: &[Profile]) {
    if !DECLARED.load(Ordering::Relaxed) {
        return;
    }
    let cur = colors();
    let saved = model::matching(profiles, &cur, &stock()).map(|p| p.name.as_str());
    let current = get_str(model::NAME_KEY);
    if let Some(name) = model::name_for_choice(listed, index, &current, saved) {
        set_str(model::NAME_KEY, &name);
    }
}

/// The dropdown index stored now (the options screen writes it on Apply).
fn stored_choice() -> i64 {
    let Some(h) = host() else {
        return model::CHOICE_CUSTOM;
    };
    let mut v = model::CHOICE_CUSTOM;
    if unsafe { (h.settings_get_i64)(h.token, s(model::PROFILE_KEY), &mut v) } != W2R_OK {
        return model::CHOICE_CUSTOM;
    }
    v
}

// ---- the profiles file -----------------------------------------------------------------------

/// `Ok(None)` when the file does not exist.
fn read_file(name: &str) -> Result<Option<String>, String> {
    let h = host().ok_or("no host")?;
    let mut handle = 0;
    match unsafe { (h.data_open)(h.token, s(name), W2R_FILE_READ, &mut handle) } {
        W2R_OK => {}
        W2R_E_NOT_FOUND => return Ok(None),
        e => return Err(format!("cannot open {name} (status {e})")),
    }
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let result = loop {
        let mut got = 0u32;
        let st = unsafe {
            (h.data_read)(
                h.token,
                handle,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut got,
            )
        };
        if st != W2R_OK {
            break Err(format!("cannot read {name} (status {st})"));
        }
        if got == 0 {
            break Ok(());
        }
        out.extend_from_slice(&chunk[..(got as usize).min(chunk.len())]);
        if out.len() > MAX_FILE {
            break Err(format!("{name} is larger than 1 MiB"));
        }
    };
    unsafe { (h.data_close)(h.token, handle) };
    result?;
    String::from_utf8(out)
        .map(Some)
        .map_err(|_| format!("{name} is not UTF-8 text"))
}

fn write_file(name: &str, text: &str) -> Result<(), String> {
    let h = host().ok_or("no host")?;
    let mut handle = 0;
    let st = unsafe { (h.data_open)(h.token, s(name), W2R_FILE_WRITE, &mut handle) };
    if st != W2R_OK {
        return Err(format!("cannot create {name} (status {st})"));
    }
    let st = unsafe { (h.data_write)(h.token, handle, text.as_ptr().cast(), text.len() as u32) };
    unsafe { (h.data_close)(h.token, handle) };
    if st == W2R_OK {
        Ok(())
    } else {
        Err(format!("cannot write {name} (status {st})"))
    }
}

/// What reading the profiles file gave: the profiles, a status line, and whether the file may be
/// written (false when it exists but could not be read).
struct Loaded {
    profiles: Vec<Profile>,
    note: String,
    file_ok: bool,
}

/// Reads `color_profiles.json`, creating it with the default profiles the first time.
fn load_profiles() -> Loaded {
    match read_file(PROFILES_FILE) {
        Ok(Some(text)) => match profiles::parse(&text) {
            Ok((profiles, warnings)) => {
                for w in warnings.iter().take(5) {
                    log(
                        W2R_LOG_WARN,
                        &format!("{PROFILES_FILE} profile {}: {}", w.entry, w.what),
                    );
                }
                let note = match warnings.first() {
                    None => format!("{} profile(s) in {PROFILES_FILE}", profiles.len()),
                    Some(w) => format!(
                        "{} profile(s); skipped {}, first: profile {} ({})",
                        profiles.len(),
                        warnings.len(),
                        w.entry,
                        w.what
                    ),
                };
                Loaded {
                    profiles,
                    note,
                    file_ok: true,
                }
            }
            Err(e) => {
                let note = format!("{PROFILES_FILE} could not be read ({e}); fix it, then Reload");
                log(W2R_LOG_WARN, &note);
                Loaded {
                    profiles: Vec::new(),
                    note,
                    file_ok: false,
                }
            }
        },
        Ok(None) => {
            let profiles = model::default_profiles();
            let note = match save_profiles(&profiles) {
                Ok(()) => format!("Created {PROFILES_FILE} with {} profiles", profiles.len()),
                Err(e) => e,
            };
            Loaded {
                profiles,
                note,
                file_ok: true,
            }
        }
        Err(e) => Loaded {
            profiles: Vec::new(),
            note: e,
            file_ok: false,
        },
    }
}

/// Writes `profiles` to `color_profiles.json`.
fn save_profiles(profiles: &[Profile]) -> Result<(), String> {
    write_file(PROFILES_FILE, &profiles::write(profiles))
}

/// Reads the game's own colours from the HD table. Called in `on_load`, before the tick has
/// written anything.
fn read_stock(game: &Game) -> [Rgb; PLAYERS] {
    let mut out = [[0u8; 3]; PLAYERS];
    for (i, rgb) in out.iter_mut().enumerate() {
        // SAFETY: the table holds four floats for each of the eight colour indices.
        *rgb = unsafe {
            let t = game.table.add(i * TABLE_STRIDE);
            model::rgb_of([*t, *t.add(1), *t.add(2)])
        };
    }
    model::stock_or_default(out)
}

// ---- the tick --------------------------------------------------------------------------------

unsafe fn read_ramp(live: *const u8) -> [Rgb; RAMP] {
    let mut out = [[0u8; 3]; RAMP];
    for (k, entry) in out.iter_mut().enumerate() {
        for (c, v) in entry.iter_mut().enumerate() {
            *v = unsafe { *live.add(k * PALETTE_STRIDE + c) };
        }
    }
    out
}

unsafe fn write_ramp(live: *mut u8, ramp: &[Rgb; RAMP]) {
    for (k, entry) in ramp.iter().enumerate() {
        for (c, v) in entry.iter().enumerate() {
            unsafe { *live.add(k * PALETTE_STRIDE + c) = *v };
        }
    }
}

/// Brings both tables in line with `wanted`. Cheap when nothing differs.
unsafe fn apply(game: &Game, wanted: &Colors) {
    let mut slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    // The render palette is not allocated early on.
    let palette = unsafe { *game.palette };
    for (i, slot) in slots.iter_mut().enumerate() {
        let target = wanted[i];
        unsafe {
            // HD table entry. Nothing is written for a colour never chosen.
            let t = game.table.add(i * TABLE_STRIDE);
            if slot.stock_table.is_none() && target.is_some() {
                slot.stock_table = Some([*t, *t.add(1), *t.add(2), *t.add(3)]);
            }
            if let Some(stock) = slot.stock_table {
                let mut want = stock;
                if let Some(rgb) = target {
                    want[..3].copy_from_slice(&model::float_rgb(rgb));
                }
                for (c, w) in want.iter().enumerate() {
                    if *t.add(c) != *w {
                        *t.add(c) = *w;
                    }
                }
            }

            // Render palette ramp (red to white only, see `model::ramp_base`).
            let Some(base) = model::ramp_base(i) else {
                continue;
            };
            if palette.is_null() {
                continue;
            }
            let live = palette.add(base * PALETTE_STRIDE);
            if slot.owns_ramp && read_ramp(live) != slot.written_ramp {
                // The palette was rebuilt: what is there now is the new stock.
                slot.owns_ramp = false;
            }
            match target {
                Some(rgb) => {
                    if !slot.owns_ramp {
                        slot.stock_ramp = read_ramp(live);
                    }
                    let ramp = model::ramp(&slot.stock_ramp, rgb);
                    if !slot.owns_ramp || read_ramp(live) != ramp {
                        write_ramp(live, &ramp);
                        slot.written_ramp = ramp;
                        slot.owns_ramp = true;
                    }
                }
                None => {
                    if slot.owns_ramp {
                        write_ramp(live, &slot.stock_ramp);
                        slot.owns_ramp = false;
                    }
                }
            }
        }
    }
}

// ---- Allied Minimap Colors -------------------------------------------------------------------

unsafe fn read8(at: *const u8) -> [u8; allied::SLOTS] {
    // SAFETY (caller): eight readable bytes; [u8; 8] needs no alignment.
    unsafe { core::ptr::read_volatile(at.cast::<[u8; allied::SLOTS]>()) }
}

/// Finds the minimap colour table next to `TeamColourRgba` and checks it before anything uses it:
/// its stock copy must hold exactly the stock dots. On any other layout (another game build) the
/// feature stays off and nothing is written.
fn setup_minimap(table: usize) -> Result<(), String> {
    let live = table
        .checked_sub(MINIMAP_BEFORE_TABLE)
        .ok_or("TeamColourRgba address too low")?;
    // SAFETY: on build 1.0.2.2818 (the only build plugin.json accepts) this is the stock minimap
    // table in the game's data section, a few kilobytes before TeamColourRgba; it is only read.
    let found = unsafe { read8((live + MINIMAP_STOCK_AFTER_LIVE) as *const u8) };
    if found != allied::STOCK {
        return Err(format!(
            "minimap color table not where build 1.0.2.2818 has it (found {found:02x?})"
        ));
    }
    let need = |name: &str| lookup(name).ok_or_else(|| format!("symbol {name} not available"));
    let m = Minimap {
        live: live as *mut u8,
        local: need("LocalPlayer")? as *const u8,
        stance: need("DiplomacyStance")? as *const u8,
        reorder: need("TeamColourReorder")? as *const u8,
        chat_mode: need("ChatTargetMode")? as *const u8,
    };
    let _ = MINIMAP.set(m);
    Ok(())
}

/// Reads the Allied Minimap Colors settings (at load and after the options screen's Apply).
fn load_allied() {
    ALLIED.store(get_i64(allied::ENABLED_KEY) == 1, Ordering::Relaxed);
    HOTKEY.store(
        allied::hotkey(get_i64(allied::HOTKEY_KEY)).1,
        Ordering::Relaxed,
    );
}

/// Turns Allied Minimap Colors on or off and stores it, so the options screen shows it too.
fn set_allied(on: bool) {
    ALLIED.store(on, Ordering::Relaxed);
    set_i64(allied::ENABLED_KEY, i64::from(on));
    log(
        W2R_LOG_INFO,
        if on {
            "Allied Minimap Colors on"
        } else {
            "Allied Minimap Colors off"
        },
    );
}

/// Whether the game window is the one in front (typing in another program does nothing).
fn game_has_focus() -> bool {
    // SAFETY: plain queries about the foreground window.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, &mut pid);
        pid == GetCurrentProcessId()
    }
}

/// Makes the live minimap table match the colours and `on`: each player's dot by their colour
/// (a recoloured Blue or White points at its palette ramp, which the recolour rewrites), then
/// allies blue and enemies red when Allied Minimap Colors is on. When that is just the game's own
/// table it is written back only if the plugin had changed it, so the game's table is never
/// touched otherwise.
unsafe fn apply_minimap(m: &Minimap, on: bool, wanted: &Colors) {
    unsafe {
        let current = read8(m.live);
        let reorder = read8(m.reorder);
        let recolored = wanted.map(|c| c.is_some());
        let ramps: [Option<u8>; allied::SLOTS] =
            core::array::from_fn(|c| model::ramp_base(c).and_then(|b| u8::try_from(b).ok()));
        let base = allied::base_table(&reorder, &current, &recolored, &ramps);
        let want = if on {
            let local = *m.local;
            if usize::from(local) < allied::SLOTS {
                let row = read8(m.stance.add(usize::from(local) * allied::STANCE_STRIDE));
                allied::allied_table(local, &row, &base).unwrap_or(base)
            } else {
                base
            }
        } else {
            base
        };
        let stock = allied::stock_table(&reorder, &current);
        let next = if want != stock {
            MINIMAP_OWNED.store(true, Ordering::Relaxed);
            want
        } else if MINIMAP_OWNED.swap(false, Ordering::Relaxed) {
            stock
        } else {
            return;
        };
        for (p, dot) in next.iter().enumerate() {
            if current[p] != *dot {
                *m.live.add(p) = *dot;
            }
        }
    }
}

/// Once per frame in a match: the toggle key, then the minimap table.
fn allied_tick() {
    let Some(m) = MINIMAP.get() else { return };
    let vk = HOTKEY.load(Ordering::Relaxed);
    if vk != 0 {
        // SAFETY: a plain key-state query; ChatTargetMode is a byte in the game's data section.
        let down = (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0;
        let focus = game_has_focus();
        let chatting = unsafe { *m.chat_mode } != 0;
        let press = KEY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .press(down, focus && !chatting);
        match press {
            allied::Press::Toggle => set_allied(!ALLIED.load(Ordering::Relaxed)),
            allied::Press::Ignored => log(
                W2R_LOG_INFO,
                if chatting {
                    "Allied Minimap Colors hotkey ignored: the chat box is open"
                } else {
                    "Allied Minimap Colors hotkey ignored: the game window is not in front"
                },
            ),
            allied::Press::None => {}
        }
    }
    unsafe { apply_minimap(m, ALLIED.load(Ordering::Relaxed), &colors()) };
}

unsafe extern "C" fn menu_buttons_detour(view: i32) {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        let original: MenuButtonsFn = unsafe { core::mem::transmute(original) };
        unsafe { original(view) };
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(game) = GAME.get() {
            unsafe { apply(game, &colors()) };
        }
        allied_tick();
    }));
}

// ---- the plugin manager card -----------------------------------------------------------------

fn fill(buf: &mut [u8], text: &str) {
    buf.fill(0);
    let n = text.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&text.as_bytes()[..n]);
}

fn text_of(buf: &[u8]) -> String {
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Shows the colours in use in the card's boxes: the chosen colour, or the game's own where
/// there is none, as the options screen does. (Not called while the player types, so a box can be
/// emptied.)
fn refill(page: &mut Page, c: &Colors) {
    let stock = stock();
    for ((buf, rgb), own) in page.hex.iter_mut().zip(c).zip(stock) {
        fill(buf, &model::to_hex(rgb.unwrap_or(own)));
    }
}

fn selected(page: &Page) -> Option<&Profile> {
    let i = usize::try_from(page.selected).ok()?.checked_sub(1)?;
    page.profiles.get(i)
}

/// Puts the selected profile's name in the name box, ready to save over it.
fn fill_names(page: &mut Page) {
    let name = selected(page).map(|p| p.name.clone()).unwrap_or_default();
    fill(&mut page.name, &name);
}

/// What the "In use" line says: Game default, the matching profile, or Custom colors.
fn in_use(page: &Page) -> String {
    let cur = colors();
    if cur == [None; PLAYERS] {
        return "Game default".into();
    }
    match model::matching(&page.profiles, &cur, &stock()) {
        Some(p) => p.name.clone(),
        None => "Custom colors".into(),
    }
}

fn new_page() -> Page {
    let loaded = load_profiles();
    let status = loaded.note;
    let mut page = Page {
        profiles: loaded.profiles,
        file_ok: loaded.file_ok,
        selected: 0,
        hex: [[0; HEX_CAP]; PLAYERS],
        name: [0; NAME_CAP],
        status,
        confirm_delete: None,
    };
    refill(&mut page, &colors());
    page
}

/// The host refuses UI strings over 128 bytes, so longer text is cut on a character boundary.
fn fit(text: &str) -> &str {
    let mut n = text.len().min(128);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    &text[..n]
}

unsafe fn label(ui: &W2rUi, text: &str) {
    unsafe { (ui.label)(ui.ctx, s(fit(text))) };
}

unsafe fn button(ui: &W2rUi, text: &str) -> bool {
    unsafe { (ui.button)(ui.ctx, s(text)) != 0 }
}

/// Applies colours chosen on the card and shows them in the boxes.
fn choose(page: &mut Page, c: Colors) {
    set_colors(c);
    refill(page, &c);
    sync_choice(&page.profiles);
}

/// Removes the profile named `name` from the file. The colours in use stay as they are.
fn delete_named(page: &mut Page, name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return "Nothing deleted: no profile name".into();
    }
    if model::is_protected(name) {
        return format!("Not deleted: {name} is built in");
    }
    if !page.file_ok {
        return format!("Not deleted: {PROFILES_FILE} could not be read; fix it, then Reload");
    }
    let mut next = page.profiles.clone();
    let Some(gone) = profiles::remove(&mut next, name) else {
        return format!("Nothing deleted: no profile named {name}");
    };
    if let Err(e) = save_profiles(&next) {
        return format!("Not deleted: {e}");
    }
    page.profiles = next;
    page.selected = page.selected.clamp(0, page.profiles.len() as i32);
    fill_names(page);
    sync_choice(&page.profiles);
    // Do not offer the deleted name again in the options screen's name field.
    if DECLARED.load(Ordering::Relaxed)
        && get_str(model::NAME_KEY)
            .trim()
            .eq_ignore_ascii_case(&gone.name)
    {
        set_str(model::NAME_KEY, "");
    }
    format!("Deleted {} from {PROFILES_FILE}", gone.name)
}

/// Saves the colours in use as a profile named by the name box.
fn save_current(page: &mut Page) -> String {
    let typed = model::clean_name(&text_of(&page.name), "");
    if typed.is_empty() {
        return "Type a profile name first".into();
    }
    // Custom and the built-in names save to the one "Custom" profile.
    let name = model::save_target(&typed);
    if !page.file_ok {
        return format!("Not saved: {PROFILES_FILE} could not be read; fix it, then Reload");
    }
    // Saving over a profile keeps the fields this plugin does not use (highlights, bars).
    let extra = page
        .profiles
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(&name))
        .map(|o| o.extra.clone())
        .unwrap_or_default();
    let p = Profile {
        name: name.clone(),
        colors: colors(),
        extra,
    };
    let mut next = page.profiles.clone();
    if let Err(e) = profiles::upsert(&mut next, p) {
        return format!("Not saved: {e}");
    }
    if let Err(e) = save_profiles(&next) {
        return format!("Not saved: {e}");
    }
    page.profiles = next;
    if let Some(i) = page.profiles.iter().position(|o| o.name == name) {
        page.selected = i as i32 + 1;
    }
    sync_choice(&page.profiles);
    prefill_name(&name);
    if name.eq_ignore_ascii_case(&typed) {
        format!("Saved {name} to {PROFILES_FILE}")
    } else {
        format!("Saved as {name} to {PROFILES_FILE} ({typed} is built in)")
    }
}

unsafe fn page_ui(ui: &W2rUi, page: &mut Page) {
    let edits = ui.struct_size >= W2R_UI_TEXT_EDIT_SIZE;
    unsafe {
        let dropdown = combo_of(ui);
        // The Color Profile dropdown shows the profile in use; without it, a line does.
        if dropdown.is_none() {
            // The host draws plain text only, so the line is set apart by its shape.
            label(ui, &format!(">>  IN USE:  {}  <<", in_use(page)));
        }

        if DECLARED.load(Ordering::Relaxed) {
            label(
                ui,
                "Also in Options > Unit Color: color pickers and the Color Profile list.",
            );
        }
        // Color swatches when the host offers them, else text boxes. The options screen's pickers
        // show a change made here the next time it opens.
        if let Some(color_edit) = color_edit_of(ui) {
            label(
                ui,
                "Click a color to change it. Picking the game's own color leaves it unchanged.",
            );
            let in_use = colors();
            let stock = stock();
            for i in 0..PLAYERS {
                let caption = format!("Player {} ({})", i + 1, model::NAMES[i]);
                let mut hex = [0u8; 7];
                let shown = model::to_setting(in_use[i].unwrap_or(stock[i]));
                hex[..6].copy_from_slice(&shown.as_bytes()[..6]);
                if color_edit(ui.ctx, s(&caption), hex.as_mut_ptr()) != 0 {
                    let picked = String::from_utf8_lossy(&hex[..6]).into_owned();
                    let mut next = colors();
                    next[i] = model::wanted(&picked, stock[i]);
                    set_colors(next);
                    refill(page, &next);
                    sync_choice(&page.profiles);
                    page.status = format!("{}: #{}", model::NAMES[i], picked.to_ascii_uppercase());
                }
            }
        } else if edits {
            label(ui, "Type #RRGGBB, or clear a box for the game's own color.");
            for i in 0..PLAYERS {
                let caption = format!("Player {} ({})", i + 1, model::NAMES[i]);
                let buf = &mut page.hex[i];
                if (ui.text_edit)(ui.ctx, s(&caption), buf.as_mut_ptr(), HEX_CAP as u32) != 0 {
                    match model::parse_entry(&text_of(buf)) {
                        Ok(c) => {
                            let mut next = colors();
                            next[i] = c;
                            set_colors(next);
                            sync_choice(&page.profiles);
                            page.status.clear();
                        }
                        Err(model::NotAColor) => {
                            page.status = format!("{}: not a #RRGGBB color yet", model::NAMES[i])
                        }
                    }
                }
            }
        } else {
            label(
                ui,
                "This w2r is too old for color boxes; use the profiles below.",
            );
        }

        // The Color Profile dropdown, as in Options > Unit Color (Custom, Game default, then the
        // first profiles in color_profiles.json), but live: it lists the file as it is now, shows
        // the profile in use, and a pick applies at once.
        if let Some(combo) = dropdown {
            if let Some(note) = profile_dropdown(ui, combo, page) {
                page.status = note;
            }
        } else {
            // A loader without card dropdowns: the slider and Apply profile. 0 is the game's own
            // colours, then color_profiles.json in file order.
            let last = page.profiles.len() as i32;
            page.selected = page.selected.clamp(0, last);
            if last > 0 && (ui.slider_int)(ui.ctx, s("Profile"), &mut page.selected, 0, last) != 0 {
                fill_names(page);
                page.confirm_delete = None;
            }
            let title =
                selected(page).map_or_else(|| "Game default".to_string(), |p| p.name.clone());
            label(ui, &format!("Profile: {title}"));
            if button(ui, "Apply profile") {
                let c = selected(page).map_or([None; PLAYERS], |p| p.colors);
                if let Some(p) = selected(page) {
                    let n = p.name.clone();
                    prefill_name(&n);
                }
                choose(page, c);
                page.status = format!("Applied {title}");
            }
        }

        if edits {
            let buf = &mut page.name;
            (ui.text_edit)(ui.ctx, s("Profile name"), buf.as_mut_ptr(), NAME_CAP as u32);
            if button(ui, "Save current colors as this profile") {
                page.status = save_current(page);
            }
        }

        // Delete the profile shown in the dropdown (the one in use), or chosen with the slider,
        // after an "are you sure". The built-in profiles cannot be deleted.
        match page.confirm_delete.clone() {
            None => {
                if button(ui, "Delete this profile") {
                    let target = if dropdown.is_some() {
                        model::matching(&page.profiles, &colors(), &stock()).map(|p| p.name.clone())
                    } else {
                        selected(page).map(|p| p.name.clone())
                    };
                    match target {
                        Some(name) if model::is_protected(&name) => {
                            page.status = format!("{name} is built in and cannot be deleted")
                        }
                        Some(name) => page.confirm_delete = Some(name),
                        None if dropdown.is_some() => {
                            page.status = "Choose a profile in the Color Profile list first".into()
                        }
                        None => page.status = "Choose a profile with the slider first".into(),
                    }
                }
            }
            Some(name) => {
                label(ui, &format!("Delete \"{name}\"? This cannot be undone."));
                if button(ui, "Yes, delete it") {
                    page.status = delete_named(page, &name);
                    page.confirm_delete = None;
                }
                if button(ui, "No, keep it") {
                    page.confirm_delete = None;
                    page.status = format!("Kept {name}");
                }
            }
        }

        if button(ui, &format!("Reload {PROFILES_FILE}")) {
            let loaded = load_profiles();
            page.profiles = loaded.profiles;
            page.file_ok = loaded.file_ok;
            page.status = loaded.note;
            page.confirm_delete = None;
        }
        if button(ui, "Reset all to game default") {
            choose(page, [None; PLAYERS]);
            page.status = "All colors back to the game's own".into();
        }

        // Allied Minimap Colors.
        if MINIMAP.get().is_some() {
            let mut on = i32::from(ALLIED.load(Ordering::Relaxed));
            if (ui.checkbox)(
                ui.ctx,
                s("Allied Minimap Colors (allies blue, enemies red)"),
                &mut on,
            ) != 0
            {
                set_allied(on != 0);
            }
            let (key, _) = allied::hotkey(get_i64(allied::HOTKEY_KEY));
            label(
                ui,
                &format!("Hotkey: {key} turns it on or off in a match (not while chatting)."),
            );
            if DECLARED.load(Ordering::Relaxed) {
                label(
                    ui,
                    "Change the hotkey in Options > Unit Color > Allied Minimap Colors.",
                );
            }
        } else {
            label(
                ui,
                "Allied Minimap Colors: not available on this game build (see the log).",
            );
        }

        if !page.status.is_empty() {
            label(ui, &page.status);
        }
        label(
            ui,
            "File: plugins\\w2r-unit-color\\data\\color_profiles.json in your w2r folder.",
        );
        // The host cuts a label at 128 bytes, so the credit takes two lines.
        label(ui, "Based on Black Recolor by Beau Hastings, a port of the");
        label(ui, "Black2Pink option of wc2r-plus by Luxray997.");
        label(
            ui,
            "Inspired by mauritsschelvis87 (War_2_Remastered_Mods) and Lambchops (Allied Colors).",
        );
    }
}

/// Draws the card's Color Profile dropdown and applies a pick: Game default, or a profile's colors
/// (the swatches, the boxes and the name fields follow at once). Returns an empty status line when
/// something was applied (the dropdown itself shows it), to clear an older message.
unsafe fn profile_dropdown(ui: &W2rUi, combo: ComboFn, page: &mut Page) -> Option<String> {
    let listed = model::listed_names(&page.profiles);
    let labels = model::choice_labels(&page.profiles);
    let strs: Vec<W2rStr> = labels.iter().map(|l| s(l)).collect();
    let cur = colors();
    let matched = model::matching(&page.profiles, &cur, &stock()).map(|p| p.name.clone());
    let mut sel = model::choice_for(&listed, cur == [None; PLAYERS], matched.as_deref()) as i32;
    let picked = unsafe {
        combo(
            ui.ctx,
            s("Color Profile"),
            strs.as_ptr(),
            strs.len() as u32,
            &mut sel,
        )
    };
    if picked == 0 {
        return None;
    }
    page.confirm_delete = None;
    // Custom applies the saved "Custom" profile, when there is one.
    let custom = model::custom_profile(&page.profiles).map(|p| p.name.clone());
    let target = match (i64::from(sel), custom.as_deref()) {
        (model::CHOICE_CUSTOM, Some(name)) => Some(Some(name)),
        (index, _) => model::choice_target(&listed, index),
    };
    match target {
        Some(None) => {
            choose(page, [None; PLAYERS]);
            page.selected = 0;
            fill_names(page);
            // The dropdown shows what was applied; clear any older message instead.
            Some(String::new())
        }
        Some(Some(name)) => {
            let i = page
                .profiles
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(name))?;
            let p = page.profiles[i].clone();
            page.selected = i as i32 + 1;
            fill_names(page);
            prefill_name(&p.name);
            choose(page, p.colors);
            Some(String::new())
        }
        // Custom without a saved Custom profile: there is nothing to apply.
        None => None,
    }
}

unsafe extern "C" fn on_ui(ui: *const W2rUi) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(ui) = (unsafe { ui.as_ref() }) else {
            return;
        };
        let mut guard = PAGE.lock().unwrap_or_else(|e| e.into_inner());
        let page = guard.get_or_insert_with(new_page);
        unsafe { page_ui(ui, page) };
    }));
}

/// The options screen stored new colours (Apply or Save and Exit).
unsafe extern "C" fn on_event(event: *const W2rEvent) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(event) = (unsafe { event.as_ref() }) else {
            return;
        };
        if event.id == W2R_EVT_SETTINGS_CHANGED {
            settings_changed();
        }
    }));
}

/// After Apply on the options screen. A new pick in the profile dropdown wins over colour edits
/// made in the same visit (the screen cannot show a profile's colours until it is opened again);
/// otherwise the colours are taken as edited and the dropdown is set to what they match.
fn settings_changed() {
    load_allied();
    let mut guard = PAGE.lock().unwrap_or_else(|e| e.into_inner());
    let page = guard.get_or_insert_with(new_page);
    let picked = stored_choice();
    let last = *LAST_CHOICE.lock().unwrap_or_else(|e| e.into_inner());
    let listed = LISTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let custom = model::custom_profile(&page.profiles).map(|p| p.name.clone());
    let profile = if picked != last {
        match (picked, custom.as_deref()) {
            // Custom applies the saved "Custom" profile, when there is one.
            (model::CHOICE_CUSTOM, Some(name)) => Some(Some(name)),
            _ => model::choice_target(&listed, picked),
        }
    } else {
        None
    };
    let next = match profile {
        Some(None) => [None; PLAYERS],
        Some(Some(name)) => match page
            .profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
        {
            Some(p) => p.colors,
            None => {
                page.status = format!("{name} is no longer in {PROFILES_FILE}");
                log(W2R_LOG_WARN, &page.status);
                load_colors()
            }
        },
        None => load_colors(),
    };
    // The screen just stored the index it shows; start from that so a later sync compares right.
    *LAST_CHOICE.lock().unwrap_or_else(|e| e.into_inner()) = picked;
    set_colors(next);
    refill(page, &next);
    if profile.is_some() {
        follow_name(&listed, picked, &page.profiles);
    }
    let checked = |key| get_i64(key) == 1;
    let (save, delete, yes) = (
        checked(model::SAVE_KEY),
        checked(model::DELETE_KEY),
        checked(model::CONFIRM_KEY),
    );
    for (on, key) in [
        (save, model::SAVE_KEY),
        (delete, model::DELETE_KEY),
        (yes, model::CONFIRM_KEY),
    ] {
        if on {
            set_i64(key, 0);
        }
    }
    let name = get_str(model::NAME_KEY);
    let note = match model::profile_action(save, delete, yes) {
        model::ProfileAction::None => None,
        model::ProfileAction::Save => {
            fill(&mut page.name, &name);
            Some(save_current(page))
        }
        model::ProfileAction::Delete => Some(delete_named(page, &name)),
        model::ProfileAction::DeleteNotConfirmed => {
            Some("Nothing deleted: check \"Yes, Delete It\" as well".to_string())
        }
        model::ProfileAction::Both => {
            Some("Save and Delete were both checked; nothing was done".to_string())
        }
    };
    if let Some(note) = note {
        log(W2R_LOG_INFO, &note);
        page.status = note;
    }
    sync_choice(&page.profiles);
    // Also when the dropdown did not change (Custom stays Custom): a name typed but not saved
    // goes back to "Custom".
    let index = *LAST_CHOICE.lock().unwrap_or_else(|e| e.into_inner());
    follow_name(&listed, index, &page.profiles);
}

/// Puts a profile's name in the options screen's Profile Name field (shown on the next visit), so
/// editing a profile is: choose it, reopen, change colors, check Save, Apply.
fn prefill_name(name: &str) {
    if !DECLARED.load(Ordering::Relaxed) {
        return;
    }
    set_str(model::NAME_KEY, name);
}

fn get_i64(key: &str) -> i64 {
    let Some(h) = host() else { return 0 };
    let mut v = 0i64;
    if unsafe { (h.settings_get_i64)(h.token, s(key), &mut v) } != W2R_OK {
        return 0;
    }
    v
}

fn set_i64(key: &str, value: i64) {
    if let Some(h) = host() {
        unsafe { (h.settings_set_i64)(h.token, s(key), value) };
    }
}

// ---- load and unload -------------------------------------------------------------------------

fn remove_hook() {
    let hook = HOOK.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let (Some(h), Some(hook)) = (host(), hook) {
        unsafe { (h.hook_remove)(h.token, hook) };
    }
}

unsafe extern "C" fn on_load(host: *const W2rHost) -> W2rStatus {
    catch_unwind(AssertUnwindSafe(|| unsafe { load(host) })).unwrap_or(W2R_E_STATE)
}

unsafe fn load(host: *const W2rHost) -> W2rStatus {
    HOST.store(host.cast_mut(), Ordering::Release);
    let (Some(table), Some(palette)) = (lookup("TeamColourRgba"), lookup("RenderPalette")) else {
        log(
            W2R_LOG_ERROR,
            "TeamColourRgba or RenderPalette not available; no recolor",
        );
        return W2R_E_STATE;
    };
    let game = GAME.get_or_init(|| Game {
        table: table as *mut f32,
        palette: palette as *const *mut u8,
    });
    // The stock colours first: they are the pickers' defaults and mean "no change".
    let stock = read_stock(game);
    *STOCK.lock().unwrap_or_else(|e| e.into_inner()) = stock;
    // Allied Minimap Colors needs the minimap table; without it the rest still works.
    match setup_minimap(table) {
        Ok(()) => log(W2R_LOG_INFO, "Allied Minimap Colors ready"),
        Err(e) => log(W2R_LOG_WARN, &format!("Allied Minimap Colors off: {e}")),
    }
    // Reads (or creates) color_profiles.json now: the dropdown lists its profiles, and a broken
    // file shows in the log at start.
    let page = new_page();
    log(W2R_LOG_INFO, &page.status);
    // Declarations are only accepted while on_load runs.
    DECLARED.store(declare(&stock, &page.profiles), Ordering::Relaxed);
    *LAST_CHOICE.lock().unwrap_or_else(|e| e.into_inner()) = stored_choice();
    load_allied();
    let wanted = load_colors();
    // Rewrite anything stored in another form (`#RRGGBB` from 0.1.0) the way the picker stores it.
    set_colors(wanted);
    sync_choice(&page.profiles);
    let mut page = page;
    refill(&mut page, &wanted);
    *PAGE.lock().unwrap_or_else(|e| e.into_inner()) = Some(page);
    let Some(h) = self::host() else {
        return W2R_E_STATE;
    };
    unsafe { (h.subscribe)(h.token, W2R_EVT_SETTINGS_CHANGED) };
    let Some(target) = handle_of("HudMenuButtons") else {
        log(
            W2R_LOG_ERROR,
            "symbol HudMenuButtons not available; no recolor",
        );
        return W2R_E_STATE;
    };
    let mut trampoline: *mut c_void = core::ptr::null_mut();
    let mut hook = 0;
    let status = unsafe {
        (h.hook_install)(
            h.token,
            target,
            menu_buttons_detour as *mut c_void,
            &mut trampoline,
            &mut hook,
        )
    };
    if status != W2R_OK {
        // W2R_E_CONFLICT: most likely Black Recolor is still enabled; it hooks the same function.
        log(
            W2R_LOG_ERROR,
            &format!("HudMenuButtons: hook_install returned {status}; no recolor"),
        );
        return W2R_E_STATE;
    }
    ORIGINAL.store(trampoline as usize, Ordering::Release);
    *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    log(W2R_LOG_INFO, "unit color ready");
    W2R_OK
}

unsafe extern "C" fn on_unload() {
    remove_hook();
    // Put the stock colours back (without touching the saved settings).
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(game) = GAME.get() {
            unsafe { apply(game, &[None; PLAYERS]) };
        }
        if let Some(m) = MINIMAP.get() {
            unsafe { apply_minimap(m, false, &[None; PLAYERS]) };
        }
    }));
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
