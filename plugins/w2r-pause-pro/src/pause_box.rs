//! The pause box (32-bit only): while a match is paused, a window in the middle of the map in the
//! race's stock dialog art (the frame of the game's Tips dialog), saying "PAUSED", who paused (their
//! colour swatch, as Player Color Bar draws it, and their name) and, online, how many pauses they
//! have used and have left.
//!
//! It is drawn in the game's dialog layer (`NkContextMenus`), which is rendered after the
//! post-effect, so the grey-out or the tint never covers it. The hook is `VictoryPopupBuild`, which
//! the match loop calls once per frame at top level in that layer's build pass; after the game's own
//! popup the box follows the same steps the victory popup does: the race's dialog skin on the dialog
//! skin manager, a window in the dialog context, a layout space, the contents, the end of the
//! window, the previous skin back.

use core::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use w2r_abi::*;

use crate::model;
use crate::plugin::{handle_of, host, log, lookup, s, setting};

type VoidFn = unsafe extern "C" fn();
type ExtentFn = unsafe extern "C" fn() -> f32;
type PopupRectFn = unsafe extern "C" fn(out: *mut f32, scale: f32) -> *mut f32;
type SetSkinFn = unsafe extern "thiscall" fn(manager: *mut c_void, skin: u32) -> u32;
type BeginWindowFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    name: *const c_char,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    flags: u32,
) -> i32;
type CtxFn = unsafe extern "C" fn(ctx: *mut c_void);
type SpaceBeginFn = unsafe extern "C" fn(ctx: *mut c_void, fmt: i32, height: f32, count: i32);
type FillRectFn = unsafe extern "C" fn(
    canvas: *mut c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rounding: f32,
    colour: u32,
);
type WidgetTextFn = unsafe extern "C" fn(
    canvas: *mut c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: *const c_char,
    len: i32,
    style: *const c_void,
    align: u32,
    font: *const c_void,
);
type TextWidthFn = unsafe extern "C" fn(*mut c_void, f32, *const c_char, i32) -> f32;

// Nuklear context and window layout of build 2818: the offsets Player Color Bar reads (the host's
// guard signatures "nk ctx->current" and "nk win->buffer" check the code that uses them).
const CTX_CURRENT_WINDOW: usize = 0x3d28;
const WINDOW_BUFFER: usize = 0x64;
const CTX_FONT: usize = 0x1e0;
const CTX_TEXT_EXTRA0: usize = 0x210;
const CTX_TEXT_EXTRA1: usize = 0x21c;
const CTX_TEXT_EXTRA2: usize = 0x220;
const CTX_WINDOW_BACKGROUND: usize = 0x1f9c;
const USER_FONT_HEIGHT: usize = 0x04;
const USER_FONT_WIDTH: usize = 0x08;
const NAME_STRIDE: usize = 0x38;
const MAX_PLAYERS: usize = 8;
// The context's window list and a window's name, flags and link (standard Nuklear, as `nk_clear`
// at 0x5a7220 walks it: begin at 0x3d1c, flags at +0x48, next at +0x17c).
const CTX_FIRST_WINDOW: usize = 0x3d1c;
const WINDOW_NAME_TEXT: usize = 0x08;
const WINDOW_NAME_MAX: usize = 64;
const WINDOW_FLAGS_AT: usize = 0x48;
const WINDOW_NEXT: usize = 0x17c;
const NK_WINDOW_HIDDEN: u32 = 0x2000;
const NK_WINDOW_CLOSED: u32 = 0x4000;
/// More windows than this is not a dialog layer we understand: no box.
const MAX_WINDOWS: usize = 64;

/// No scrollbar (as the victory popup), drawn behind the layer's other windows, and never takes
/// the mouse or the keyboard (chat and the HUD keep working while paused).
const WINDOW_FLAGS: u32 = 0x20 | 0x100 | 0x400;
const WINDOW_NAME: &core::ffi::CStr = c"w2r_pause_box";
/// Nuklear text alignment: middle, and centred or left.
const ALIGN_CENTRE: u32 = 0x12;
const ALIGN_LEFT: u32 = 0x11;
/// Gold title, white text; the swatch outline as Player Color Bar draws it.
const TITLE_COLOUR: u32 = 0xff4c_d3ff;
const TEXT_COLOUR: u32 = 0xffff_ffff;
const BORDER: u32 = 0xc800_0000;
const BORDER_SELF: u32 = 0xffff_ffff;

struct BoxGame {
    popup_rect: PopupRectFn,
    space_width: ExtentFn,
    hud_width: ExtentFn,
    hud_height: ExtentFn,
    set_skin: SetSkinFn,
    begin_window: BeginWindowFn,
    end_window: CtxFn,
    space_begin: SpaceBeginFn,
    space_end: CtxFn,
    fill_rect: FillRectFn,
    widget_text: WidgetTextFn,
    menus_ctx: *const *mut c_void,
    skin_manager: *mut c_void,
    skin_human: *const u32,
    skin_orc: *const u32,
    skin_alt: *const u8,
    pause_flags: *const u8,
    pauses_remaining: *const u8,
    player_names: *const u8,
    command_issuer: *const u8,
    local_player: *const u8,
    networked_match: *const u8,
    colour_rgba: *const f32,
    colour_reorder: *const u8,
    /// `GraphicsMode` (0 classic, 1 HD), null when the host does not have it (HD assumed).
    graphics_mode: *const u32,
}

// SAFETY: addresses of the game's globals and functions, fixed for the session; used from the
// game's main thread.
unsafe impl Send for BoxGame {}
unsafe impl Sync for BoxGame {}

static GAME: OnceLock<Result<BoxGame, &'static str>> = OnceLock::new();
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);
static SHOW: AtomicBool = AtomicBool::new(true);
/// Index into `model::BOX_POSITIONS`.
static POSITION: AtomicUsize = AtomicUsize::new(model::BOX_POSITION_DEFAULT as usize);
/// The slot of the player whose pause is showing (recorded by the pause handler), -1 for none.
static PAUSER: AtomicI32 = AtomicI32::new(-1);
/// The position and graphics mode last logged (`usize::MAX` before the first box), so each change
/// is logged once.
static LOGGED_POSITION: AtomicUsize = AtomicUsize::new(usize::MAX);
/// The game's window (found from the drawing thread), 0 before it is found.
static WINDOW: AtomicUsize = AtomicUsize::new(0);
/// The window the box last gave way to, so each change is logged once.
static GAVE_WAY_TO: Mutex<String> = Mutex::new(String::new());

/// The name of a visible window in the dialog layer other than the box (a menu, the Tips dialog,
/// Alliances, ...), from the windows of the last frame; `Some("?")` when the list looks wrong.
/// The box is not drawn while one is open.
fn other_dialog(ctx: *mut c_void) -> Option<String> {
    unsafe {
        let mut window = ctx
            .cast::<u8>()
            .add(CTX_FIRST_WINDOW)
            .cast::<*const u8>()
            .read_unaligned();
        let mut seen = 0;
        while !window.is_null() {
            seen += 1;
            if seen > MAX_WINDOWS {
                return Some("?".into());
            }
            let flags = window.add(WINDOW_FLAGS_AT).cast::<u32>().read_unaligned();
            let raw = core::slice::from_raw_parts(window.add(WINDOW_NAME_TEXT), WINDOW_NAME_MAX);
            let len = raw.iter().position(|&b| b == 0).unwrap_or(WINDOW_NAME_MAX);
            let name = &raw[..len];
            if flags & (NK_WINDOW_HIDDEN | NK_WINDOW_CLOSED) == 0 && name != WINDOW_NAME.to_bytes()
            {
                return Some(String::from_utf8_lossy(name).into_owned());
            }
            window = window.add(WINDOW_NEXT).cast::<*const u8>().read_unaligned();
        }
        None
    }
}

/// Logs which window the box gives way to (or that it shows again), once per change.
fn note_gave_way(to: Option<&str>) {
    let mut last = GAVE_WAY_TO.lock().unwrap_or_else(|e| e.into_inner());
    let now = to.unwrap_or("");
    if *last != now {
        if now.is_empty() {
            log(W2R_LOG_INFO, "pause box: shown");
        } else {
            log(
                W2R_LOG_INFO,
                &format!("pause box: hidden while the dialog window \"{now}\" is open"),
            );
        }
        *last = now.to_string();
    }
}

fn resolve() -> Result<BoxGame, &'static str> {
    let at = |name: &'static str| lookup(name).ok_or(name);
    // SAFETY (each transmute): the symbol is the function the host's table declares, with the
    // signature written here.
    unsafe {
        Ok(BoxGame {
            popup_rect: core::mem::transmute::<usize, PopupRectFn>(at("DialogPopupRect")?),
            space_width: core::mem::transmute::<usize, ExtentFn>(at("GetUiSpaceWidth")?),
            hud_width: core::mem::transmute::<usize, ExtentFn>(at("HudUiWidth")?),
            hud_height: core::mem::transmute::<usize, ExtentFn>(at("HudUiHeight")?),
            set_skin: core::mem::transmute::<usize, SetSkinFn>(at("SetSkin")?),
            begin_window: core::mem::transmute::<usize, BeginWindowFn>(at("BeginNamedWindow")?),
            end_window: core::mem::transmute::<usize, CtxFn>(at("EndCurrentWindow")?),
            space_begin: core::mem::transmute::<usize, SpaceBeginFn>(at("LayoutSpaceBegin")?),
            space_end: core::mem::transmute::<usize, CtxFn>(at("LayoutSpaceEnd")?),
            fill_rect: core::mem::transmute::<usize, FillRectFn>(at("NkFillRect")?),
            widget_text: core::mem::transmute::<usize, WidgetTextFn>(at("NkWidgetText")?),
            menus_ctx: at("NkContextMenus")? as *const *mut c_void,
            skin_manager: at("DialogSkinManager")? as *mut c_void,
            skin_human: at("StockPopupSkinHuman")? as *const u32,
            skin_orc: at("StockPopupSkinOrc")? as *const u32,
            skin_alt: at("SkinAltFlag")? as *const u8,
            pause_flags: at("MatchPauseFlags")? as *const u8,
            pauses_remaining: at("PausesRemaining")? as *const u8,
            player_names: at("PlayerNames")? as *const u8,
            command_issuer: at("CommandIssuer")? as *const u8,
            local_player: at("LocalPlayer")? as *const u8,
            networked_match: at("NetworkedMatch")? as *const u8,
            colour_rgba: at("TeamColourRgba")? as *const f32,
            colour_reorder: at("TeamColourReorder")? as *const u8,
            graphics_mode: lookup("GraphicsMode").unwrap_or(0) as *const u32,
        })
    }
}

fn game() -> Option<&'static BoxGame> {
    GAME.get_or_init(resolve).as_ref().ok()
}

/// The "Pause Box" setting itself (Recolor shows the box whatever it says: see `shown`).
pub fn show() -> bool {
    SHOW.load(Ordering::Relaxed)
}

pub fn set_show(on: bool) {
    SHOW.store(on, Ordering::Relaxed);
}

/// Whether the match is paused now (the game's pause flag).
pub fn paused() -> bool {
    game().is_some_and(|g| unsafe { *g.pause_flags } & 1 != 0)
}

/// The slot of the player whose pause it is, as recorded when the pause started.
pub fn pauser() -> Option<u8> {
    u8::try_from(PAUSER.load(Ordering::Relaxed))
        .ok()
        .filter(|&p| (p as usize) < MAX_PLAYERS)
}

/// Whether the box is drawn while paused.
pub fn shown() -> bool {
    model::box_shown(show(), crate::plugin::screen())
}

pub fn position() -> usize {
    POSITION.load(Ordering::Relaxed)
}

pub fn set_position(index: usize) {
    POSITION.store(model::box_position(index as i64), Ordering::Relaxed);
}

pub fn load_setting() {
    set_show(setting(model::BOX_KEY, 1) != 0);
    set_position(model::box_position(setting(
        model::BOX_POSITION_KEY,
        model::BOX_POSITION_DEFAULT,
    )));
}

/// The pause handler asked for the grey-out: remember whose pause it is (online the issuer of
/// the pause command being handled, offline the local player).
pub fn note_pause() {
    let Some(g) = game() else { return };
    let networked = unsafe { *g.networked_match } != 0;
    let who = model::pauser(networked, unsafe { *g.command_issuer }, unsafe {
        *g.local_player
    });
    PAUSER.store(who.map_or(-1, i32::from), Ordering::Relaxed);
}

/// "Pause Box" for the game's options screen, next to "Pause Screen".
pub fn declare(h: &W2rHost) -> bool {
    if h.struct_size < W2R_HOST_SETTINGS_DECLARE_SIZE {
        return false;
    }
    let def = W2rSettingDef {
        struct_size: W2R_SETTING_DEF_V1_SIZE,
        kind: W2R_SETTING_BOOL,
        default_int: 1,
        min: 0,
        max: 0,
        flags: 0,
        choice_count: 0,
        choices: core::ptr::null(),
        reserved: 0,
        key: s(model::BOX_KEY),
        label: s(model::BOX_LABEL),
        description: s(model::BOX_DESCRIPTION),
        category: s(model::CATEGORY),
        subcategory: s(model::SUBCATEGORY),
        default_str: s(""),
        active_when: s(""),
    };
    // Recolor always shows the box: on a host that can lock a checkbox (`key!=N:V`), the row
    // is greyed and shows ticked while Recolor is chosen on the screen. Any other host refuses the
    // condition, and the row is declared plain (Apply still stores it ticked: `reconcile_box`).
    let lock = model::box_lock_condition();
    let mut status = unsafe {
        (h.settings_declare)(
            h.token,
            &W2rSettingDef {
                active_when: s(&lock),
                ..def
            },
        )
    };
    if status == W2R_E_INVALID || status == W2R_E_NOT_FOUND {
        status = unsafe { (h.settings_declare)(h.token, &def) };
    }
    if status != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("declaring {}: status {status}", model::BOX_KEY),
        );
    }
    let labels: Vec<W2rStr> = model::BOX_POSITIONS.iter().map(|l| s(l)).collect();
    let position = W2rSettingDef {
        kind: W2R_SETTING_CHOICE,
        default_int: model::BOX_POSITION_DEFAULT,
        choice_count: labels.len() as u32,
        choices: labels.as_ptr(),
        key: s(model::BOX_POSITION_KEY),
        label: s(model::BOX_POSITION_LABEL),
        description: s(model::BOX_POSITION_DESCRIPTION),
        ..def
    };
    let placed = unsafe { (h.settings_declare)(h.token, &position) };
    if placed != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("declaring {}: status {placed}", model::BOX_POSITION_KEY),
        );
    }
    status == W2R_OK && placed == W2R_OK
}

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumThreadWindows(
        thread: u32,
        callback: unsafe extern "system" fn(*mut c_void, isize) -> i32,
        data: isize,
    ) -> i32;
    fn IsWindow(window: *mut c_void) -> i32;
    fn IsWindowVisible(window: *mut c_void) -> i32;
    fn GetClientRect(window: *mut c_void, rect: *mut [i32; 4]) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

fn client_area(window: *mut c_void) -> i64 {
    let mut r = [0i32; 4];
    if unsafe { GetClientRect(window, &mut r) } == 0 {
        return 0;
    }
    i64::from(r[2] - r[0]) * i64::from(r[3] - r[1])
}

/// `EnumThreadWindows` callback: keeps the largest visible window in `*data`.
unsafe extern "system" fn keep_largest(window: *mut c_void, data: isize) -> i32 {
    let best = data as *mut usize;
    if unsafe { IsWindowVisible(window) } != 0
        && client_area(window) > client_area(unsafe { *best } as *mut c_void)
    {
        unsafe { *best = window as usize };
    }
    1
}

/// The game window's height over its width: the largest visible window of the drawing thread (the
/// game's main thread, which owns it). `None` when there is none.
fn window_aspect() -> Option<f32> {
    let mut window = WINDOW.load(Ordering::Relaxed);
    if window == 0 || unsafe { IsWindow(window as *mut c_void) } == 0 {
        window = 0;
        let data = &mut window as *mut usize as isize;
        unsafe { EnumThreadWindows(GetCurrentThreadId(), keep_largest, data) };
        WINDOW.store(window, Ordering::Relaxed);
    }
    let mut r = [0i32; 4];
    if window == 0 || unsafe { GetClientRect(window as *mut c_void, &mut r) } == 0 {
        return None;
    }
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    (w > 0 && h > 0).then(|| h as f32 / w as f32)
}

fn text_width(font: *const u8, text: &str) -> f32 {
    unsafe {
        let userdata = font.cast::<*mut c_void>().read_unaligned();
        let height = font.add(USER_FONT_HEIGHT).cast::<f32>().read_unaligned();
        match font
            .add(USER_FONT_WIDTH)
            .cast::<Option<TextWidthFn>>()
            .read_unaligned()
        {
            Some(width) => width(userdata, height, text.as_ptr().cast(), text.len() as i32),
            None => text.len() as f32 * height * 0.5,
        }
    }
}

/// The pauser's swatch colour (the game's own team colour table, as Player Color Bar reads it).
fn colour_of(g: &BoxGame, who: u8) -> u32 {
    unsafe {
        let index = (*g.colour_reorder.add(who as usize) as usize).min(MAX_PLAYERS - 1);
        let c = g.colour_rgba.add(index * 4);
        model::pack([*c, *c.add(1), *c.add(2)])
    }
}

/// One frame of the box, after the victory popup in the dialog layer's build pass.
fn draw(g: &BoxGame) {
    unsafe {
        let ctx = *g.menus_ctx;
        if ctx.is_null() {
            return;
        }
        // Never over a menu, the Tips dialog, Alliances or any other dialog of the match.
        let other = other_dialog(ctx);
        note_gave_way(other.as_deref());
        if other.is_some() {
            return;
        }
        let networked = *g.networked_match != 0;
        let recorded = PAUSER.load(Ordering::Relaxed);
        let who = if recorded >= 0 {
            Some(recorded as u8)
        } else {
            model::pauser(networked, *g.command_issuer, *g.local_player)
        };
        let Some(who) = who.filter(|&w| (w as usize) < MAX_PLAYERS) else {
            return;
        };
        let raw = core::slice::from_raw_parts(
            g.player_names.add(who as usize * NAME_STRIDE),
            NAME_STRIDE,
        );
        let name = model::display_name(raw, who, networked);
        let count = networked.then(|| model::pauses_line(*g.pauses_remaining.add(who as usize)));

        let title = crate::countdown::title();
        let title = title.as_deref().unwrap_or(model::BOX_TITLE);

        let skin = if *g.skin_alt != 0 {
            *g.skin_orc
        } else {
            *g.skin_human
        };
        let previous = (g.set_skin)(g.skin_manager, skin);
        let font = ctx
            .cast::<u8>()
            .add(CTX_FONT)
            .cast::<*const u8>()
            .read_unaligned();
        if !font.is_null() {
            let font_h = font.add(USER_FONT_HEIGHT).cast::<f32>().read_unaligned();
            let (title_w, name_w) = (text_width(font, title), text_width(font, &name));
            let count_w = count.as_deref().map(|c| text_width(font, c));
            let (w, h) = model::box_size(font_h, title_w, name_w, count_w);
            let position = position();
            let mode = if g.graphics_mode.is_null() {
                1
            } else {
                g.graphics_mode.read_volatile()
            };
            let rects = model::hud_rects(
                (g.space_width)(),
                window_aspect(),
                (g.hud_width)(),
                (g.hud_height)(),
                model::layout_for(mode),
            );
            let (x, y) = match &rects {
                Some(r) => model::place_box(position, r, w, h, (font_h * 0.5).round()),
                None => {
                    // HUD sizes that make no sense: the centre of the game's own dialog place.
                    let mut popup = [0f32; 4];
                    (g.popup_rect)(popup.as_mut_ptr(), 1.0);
                    (
                        (popup[0] + (popup[2] - w) / 2.0).round(),
                        (popup[1] + (popup[3] - h) / 2.0).round(),
                    )
                }
            };
            let layout = model::box_layout(x, y, font_h, title_w, name_w, count_w);
            let logged = position + mode as usize * 100;
            if LOGGED_POSITION.swap(logged, Ordering::Relaxed) != logged {
                log(
                    W2R_LOG_INFO,
                    &format!(
                        "pause box: {} at {:?} ({} view), font height {font_h}, HUD rects {rects:?}",
                        model::BOX_POSITIONS[position],
                        layout.rect,
                        if mode == 0 { "classic" } else { "HD" }
                    ),
                );
            }
            let [x, y, w, h] = layout.rect;
            if (g.begin_window)(ctx, WINDOW_NAME.as_ptr(), x, y, w, h, WINDOW_FLAGS) != 0 {
                (g.space_begin)(ctx, 1, 0.0, 1000);
                let window = ctx
                    .cast::<u8>()
                    .add(CTX_CURRENT_WINDOW)
                    .cast::<*mut u8>()
                    .read_unaligned();
                if !window.is_null() {
                    let canvas = window.add(WINDOW_BUFFER).cast::<c_void>();
                    let mut style = [0u32; 7];
                    style[2] = ctx
                        .cast::<u8>()
                        .add(CTX_WINDOW_BACKGROUND)
                        .cast::<u32>()
                        .read_unaligned();
                    style[4] = ctx
                        .cast::<u8>()
                        .add(CTX_TEXT_EXTRA0)
                        .cast::<u32>()
                        .read_unaligned();
                    style[5] = ctx
                        .cast::<u8>()
                        .add(CTX_TEXT_EXTRA1)
                        .cast::<u32>()
                        .read_unaligned();
                    style[6] = ctx
                        .cast::<u8>()
                        .add(CTX_TEXT_EXTRA2)
                        .cast::<u32>()
                        .read_unaligned();
                    let text =
                        |rect: [f32; 4], s: &str, colour: u32, align: u32, style: &mut [u32; 7]| {
                            style[3] = colour;
                            (g.widget_text)(
                                canvas,
                                rect[0],
                                rect[1],
                                rect[2],
                                rect[3],
                                s.as_ptr().cast(),
                                s.len() as i32,
                                style.as_ptr().cast(),
                                align,
                                font.cast(),
                            );
                        };
                    text(layout.title, title, TITLE_COLOUR, ALIGN_CENTRE, &mut style);
                    let [sx, sy, sw, sh] = layout.swatch;
                    let border = if who == *g.local_player {
                        BORDER_SELF
                    } else {
                        BORDER
                    };
                    (g.fill_rect)(canvas, sx - 1.0, sy - 1.0, sw + 2.0, sh + 2.0, 0.0, border);
                    (g.fill_rect)(canvas, sx, sy, sw, sh, 0.0, colour_of(g, who));
                    text(layout.name, &name, TEXT_COLOUR, ALIGN_LEFT, &mut style);
                    if let (Some(rect), Some(line)) = (layout.count, count.as_deref()) {
                        text(rect, line, TEXT_COLOUR, ALIGN_CENTRE, &mut style);
                    }
                }
                (g.space_end)(ctx);
                (g.end_window)(ctx);
            }
        }
        (g.set_skin)(g.skin_manager, previous);
    }
}

/// The detour for `VictoryPopupBuild`: the game's popup first, then the box while paused.
unsafe extern "C" fn victory_popup_detour() {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        let original: VoidFn = unsafe { core::mem::transmute(original) };
        unsafe { original() };
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        crate::countdown::tick();
        // With the box off the countdown is in the message area instead (`countdown::write_line`).
        if shown()
            && let Some(g) = game()
            && unsafe { *g.pause_flags } & 1 != 0
        {
            draw(g);
        }
    }));
}

/// Hooks `VictoryPopupBuild`; a reason when the box is not available (older host, hook refused).
pub fn install(h: &W2rHost) -> Result<(), String> {
    if let Err(name) = GAME.get_or_init(resolve) {
        return Err(format!("symbol {name} not available (host too old?)"));
    }
    let target = handle_of("VictoryPopupBuild").ok_or("symbol VictoryPopupBuild not available")?;
    let mut trampoline: *mut c_void = core::ptr::null_mut();
    let mut hook = 0;
    let status = unsafe {
        (h.hook_install)(
            h.token,
            target,
            victory_popup_detour as *mut c_void,
            &mut trampoline,
            &mut hook,
        )
    };
    if status != W2R_OK {
        return Err(format!("VictoryPopupBuild: hook_install returned {status}"));
    }
    ORIGINAL.store(trampoline as usize, Ordering::Release);
    *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    Ok(())
}

pub fn remove() {
    let hook = HOOK.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let (Some(h), Some(hook)) = (host(), hook) {
        unsafe { (h.hook_remove)(h.token, hook) };
    }
}
