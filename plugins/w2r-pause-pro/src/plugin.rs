//! The game-facing half (32-bit only): one hook on `SetPostEffect`, the game's switch for its
//! full-screen post-effect shader.
//!
//! - Default: every mode reaches the game unchanged (the stock grey-out).
//! - Recolor: a request for the grey mode is passed on as "off".
//! - Tint: once, the plugin builds a tinted grey program with the game's own builder and puts it
//!   in the post-effect object's mode-2 slot (stock: a red-only grey that nothing asks for); at
//!   each pause it sets the tint to the pauser's colour and passes mode 2 instead of 1. If the
//!   program cannot be built it says why in the log and the stock grey stays.
//!
//! The resume countdown is in `countdown`, the box in `pause_box`. Display only: the pause flag,
//! the turn flow and every other player's game are untouched (the countdown sends the stock resume
//! command later). The settings are declared for the game's options screen and are also on the
//! plugin's card.

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicUsize, Ordering};

use w2r_abi::*;

use crate::model::{self, PausedScreen};

type SetPostEffectFn = unsafe extern "C" fn(mode: i32);
type BuildProgramFn = unsafe extern "C" fn(
    out: *mut usize,
    vertex: *const c_char,
    fragment: *const c_char,
) -> *mut usize;
type SetIntFn = unsafe extern "thiscall" fn(program: usize, name: *const c_char, value: i32);
type SetVec3Fn = unsafe extern "thiscall" fn(program: usize, name: *const c_char, xyz: *const f32);

/// `W2rUi.color_edit` and `W2rUi.combo`: appended to `W2rUi` by a W2R Mod Loader with card widgets
/// (hastinbe/w2r-mod-loader#5); the w2r-abi this workspace pins predates them, so they are declared
/// here (as Unit Color does) and used only when `struct_size` says the host has them. `combo`: a
/// caption and a dropdown of `count` choices showing `*selected`; 1 when the player picked another.
type ColorEditFn = unsafe extern "C" fn(*mut c_void, W2rStr, *mut u8) -> i32;
type ComboFn = unsafe extern "C" fn(*mut c_void, W2rStr, *const W2rStr, u32, *mut i32) -> i32;

/// `W2rUi` with the card widget fields appended.
#[repr(C)]
struct UiExt {
    base: W2rUi,
    color_edit: ColorEditFn,
    combo: ComboFn,
}

/// The host's `combo`, when its widget table is long enough to have one.
fn combo_of(ui: &W2rUi) -> Option<ComboFn> {
    let end = core::mem::offset_of!(UiExt, combo) + core::mem::size_of::<usize>();
    if (ui.struct_size as usize) < end {
        return None;
    }
    // SAFETY: the host says its table is at least this long, and the field is appended after the
    // ones `W2rUi` declares.
    Some(unsafe { (*(ui as *const W2rUi).cast::<UiExt>()).combo })
}

#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglGetCurrentContext() -> *mut c_void;
}

/// The post-effect object: the program that mode 2 draws with (behind a unique_ptr, which is the
/// bare pointer). Checked by the host's guard signature of the effect's draw method.
const MODE2_PROGRAM: usize = 0x40;
const MAX_PLAYERS: usize = 8;

/// Game addresses for Tint, from the host's symbols (build 2818).
struct Game {
    post_effect: *const usize,
    vertex_source: *const c_char,
    build_program: BuildProgramFn,
    set_int: SetIntFn,
    set_vec3: SetVec3Fn,
    command_issuer: *const u8,
    local_player: *const u8,
    networked_match: *const u8,
    colour_rgba: *const f32,
    colour_reorder: *const u8,
}

// SAFETY: addresses of the game's globals and functions, fixed for the session; used from the
// game's main thread.
unsafe impl Send for Game {}
unsafe impl Sync for Game {}

/// The tint program once built: the program and the object slot it went into, with what the slot
/// held before (put back on unload).
#[derive(Clone, Copy)]
struct Tint {
    program: usize,
    slot: usize,
    previous: usize,
}

static HOST: AtomicPtr<W2rHost> = AtomicPtr::new(core::ptr::null_mut());
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);
/// `PausedScreen::index()` of the current choice.
static SCREEN: AtomicI64 = AtomicI64::new(0);
static GAME: OnceLock<Result<Game, &'static str>> = OnceLock::new();
static TINT: Mutex<Option<Tint>> = Mutex::new(None);
/// Set when building failed for good, so it is not retried (and logged) at every pause.
static TINT_FAILED: AtomicBool = AtomicBool::new(false);

pub(crate) fn host() -> Option<&'static W2rHost> {
    unsafe { HOST.load(Ordering::Acquire).as_ref() }
}

pub(crate) fn s(text: &str) -> W2rStr {
    W2rStr {
        ptr: text.as_ptr().cast(),
        len: text.len() as u32,
    }
}

pub(crate) fn log(level: u32, msg: &str) {
    if let Some(h) = host() {
        unsafe { (h.log)(h.token, level, s(msg)) };
    }
}

pub(crate) fn handle_of(name: &str) -> Option<W2rHandle> {
    let h = host()?;
    let mut handle = 0;
    (unsafe { (h.symbol_lookup)(h.token, s(name), &mut handle) } == W2R_OK).then_some(handle)
}

pub(crate) fn lookup(name: &str) -> Option<usize> {
    let h = host()?;
    let handle = handle_of(name)?;
    let mut addr = 0usize;
    (unsafe { (h.symbol_address)(h.token, handle, &mut addr) } == W2R_OK).then_some(addr)
}

pub(crate) fn setting(key: &str, default: i64) -> i64 {
    let mut v = 0i64;
    match host() {
        Some(h) if unsafe { (h.settings_get_i64)(h.token, s(key), &mut v) } == W2R_OK => v,
        _ => default,
    }
}

pub(crate) fn screen() -> PausedScreen {
    PausedScreen::from_index(SCREEN.load(Ordering::Relaxed))
}

fn store(key: &str, value: i64) {
    if let Some(h) = host() {
        unsafe { (h.settings_set_i64)(h.token, s(key), value) };
    }
}

/// Keeps the stored Pause Box setting in step with Recolor, which always shows the box: while
/// Recolor is chosen the box is stored as on (so the options screen shows it ticked, and an untick
/// there is put back). Leaving Recolor keeps that last value: the box stays on until the player
/// turns it off.
fn reconcile_box() {
    if screen() == PausedScreen::Recolor && !crate::pause_box::show() {
        crate::pause_box::set_show(true);
        store(model::BOX_KEY, 1);
    }
}

fn load_screen() {
    let stored = setting(model::PAUSED_SCREEN_KEY, PausedScreen::DEFAULT.index());
    SCREEN.store(PausedScreen::from_index(stored).index(), Ordering::Relaxed);
}

/// The symbols Tint needs; the name of the first missing one otherwise (a host without them).
fn resolve() -> Result<Game, &'static str> {
    let at = |name: &'static str| lookup(name).ok_or(name);
    Ok(Game {
        post_effect: at("PostEffect")? as *const usize,
        vertex_source: at("PostEffectVertexSource")? as *const c_char,
        build_program: unsafe {
            core::mem::transmute::<usize, BuildProgramFn>(at("BuildShaderProgram")?)
        },
        set_int: unsafe { core::mem::transmute::<usize, SetIntFn>(at("ShaderSetInt")?) },
        set_vec3: unsafe { core::mem::transmute::<usize, SetVec3Fn>(at("ShaderSetVec3")?) },
        command_issuer: at("CommandIssuer")? as *const u8,
        local_player: at("LocalPlayer")? as *const u8,
        networked_match: at("NetworkedMatch")? as *const u8,
        colour_rgba: at("TeamColourRgba")? as *const f32,
        colour_reorder: at("TeamColourReorder")? as *const u8,
    })
}

fn game() -> Option<&'static Game> {
    GAME.get_or_init(resolve).as_ref().ok()
}

/// Builds the tint program and puts it into the mode-2 slot, once. Only with a GL context
/// current on this thread (the game's builder aborts the game when compiling fails, and it would
/// fail without one); otherwise it is tried again later. Returns whether the program is in place.
fn ensure_tint(g: &Game) -> bool {
    let mut tint = TINT.lock().unwrap_or_else(|e| e.into_inner());
    if tint.is_some() {
        return true;
    }
    if TINT_FAILED.load(Ordering::Relaxed) {
        return false;
    }
    if unsafe { wglGetCurrentContext() }.is_null() {
        return false;
    }
    let object = unsafe { *g.post_effect };
    if object == 0 {
        return false;
    }
    let mut out = 0usize;
    unsafe {
        (g.build_program)(
            &mut out,
            g.vertex_source,
            model::TINT_FRAGMENT.as_ptr().cast(),
        )
    };
    if out == 0 {
        TINT_FAILED.store(true, Ordering::Relaxed);
        log(
            W2R_LOG_ERROR,
            "Tint: the game built no program; the grey-out stays",
        );
        return false;
    }
    unsafe { (g.set_int)(out, model::IMAGE_UNIFORM.as_ptr().cast(), 0) };
    let slot = object + MODE2_PROGRAM;
    let previous = unsafe { *(slot as *const usize) };
    unsafe { *(slot as *mut usize) = out };
    *tint = Some(Tint {
        program: out,
        slot,
        previous,
    });
    log(W2R_LOG_INFO, "Tint: program built");
    true
}

/// Tint was chosen (at load, on the options screen or on the card): build its program now, while
/// a frame is being built and a GL context is current, rather than in the pause handler. Only
/// then, so a game that never uses Tint never compiles the shader. True when it is in place.
fn tint_chosen() -> bool {
    screen() == PausedScreen::Tint
        && std::panic::catch_unwind(|| game().is_some_and(ensure_tint)).unwrap_or(false)
}

/// The pauser's team colour, scaled for the tint (white when there is no pauser).
fn pauser_tint(g: &Game) -> [f32; 3] {
    // The pauser recorded at the pause (the issuer of a later command is someone else when the
    // choice changes during the pause); the issuer now when there is none.
    let networked = unsafe { *g.networked_match } != 0;
    let who = crate::pause_box::pauser().or_else(|| {
        model::pauser(networked, unsafe { *g.command_issuer }, unsafe {
            *g.local_player
        })
    });
    let Some(who) = who else {
        return [1.0, 1.0, 1.0];
    };
    unsafe {
        let index = (*g.colour_reorder.add(who as usize) as usize).min(MAX_PLAYERS - 1);
        let c = g.colour_rgba.add(index * 4);
        model::tint_colour([*c, *c.add(1), *c.add(2)])
    }
}

/// Tint at a pause: the program in place and its colour set. False falls back to the stock grey.
fn prepare_tint() -> bool {
    let Some(g) = game() else { return false };
    if !ensure_tint(g) {
        return false;
    }
    let Some(t) = *TINT.lock().unwrap_or_else(|e| e.into_inner()) else {
        return false;
    };
    let rgb = pauser_tint(g);
    unsafe { (g.set_vec3)(t.program, model::TINT_UNIFORM.as_ptr().cast(), rgb.as_ptr()) };
    true
}

/// Called by the game on the main thread (the pause and resume handlers, the end of a paused
/// match). Passes the mode on, changed per the Paused Screen choice.
unsafe extern "C" fn set_post_effect_detour(mode: i32) {
    if mode == model::GREY {
        let _ = std::panic::catch_unwind(crate::pause_box::note_pause);
    }
    apply(mode);
}

/// Passes `mode` to the game's own `SetPostEffect`, changed per the Paused Screen choice.
fn apply(mode: i32) {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    let original: SetPostEffectFn = unsafe { core::mem::transmute(original) };
    let choice = screen();
    let tint_ready = choice == PausedScreen::Tint
        && mode == model::GREY
        && std::panic::catch_unwind(prepare_tint).unwrap_or(false);
    unsafe { original(model::effect_mode(mode, choice, tint_ready)) };
}

/// The Paused Screen choice changed (options screen or card, main thread): while the match is
/// paused, the screen changes now instead of at the next pause. The same request the pause made
/// (the grey-out), answered for the new choice; the pause, its count and the other players are
/// untouched.
fn reapply_while_paused() {
    let _ = std::panic::catch_unwind(|| {
        if crate::pause_box::paused() {
            apply(model::GREY);
        }
    });
}

/// Puts "Paused Screen" on the game's options screen. False on an older host (the card still
/// has it) or when the host refused it (logged).
fn declare() -> bool {
    let Some(h) = host() else { return false };
    if h.struct_size < W2R_HOST_SETTINGS_DECLARE_SIZE {
        return false;
    }
    let labels: Vec<W2rStr> = model::CHOICES.iter().map(|l| s(l)).collect();
    let def = W2rSettingDef {
        struct_size: W2R_SETTING_DEF_V1_SIZE,
        kind: W2R_SETTING_CHOICE,
        default_int: PausedScreen::DEFAULT.index(),
        min: 0,
        max: 0,
        flags: 0,
        choice_count: labels.len() as u32,
        choices: labels.as_ptr(),
        reserved: 0,
        key: s(model::PAUSED_SCREEN_KEY),
        label: s(model::PAUSED_SCREEN_LABEL),
        description: s(model::PAUSED_SCREEN_DESCRIPTION),
        // Under its own heading in the UI category (UI, Pause Pro UI).
        category: s(model::CATEGORY),
        subcategory: s(model::SUBCATEGORY),
        default_str: s(""),
        active_when: s(""),
    };
    let status = unsafe { (h.settings_declare)(h.token, &def) };
    if status != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("declaring {}: status {status}", model::PAUSED_SCREEN_KEY),
        );
    }
    status == W2R_OK
}

/// The options screen stored a new choice (Apply or Save and Exit), or another player's copy sent
/// a countdown message.
unsafe extern "C" fn on_event(event: *const W2rEvent) {
    let Some(event) = (unsafe { event.as_ref() }) else {
        return;
    };
    if event.id == W2R_EVT_SETTINGS_CHANGED {
        let before = screen();
        load_screen();
        crate::pause_box::load_setting();
        crate::countdown::load_setting();
        reconcile_box();
        tint_chosen();
        if screen() != before {
            reapply_while_paused();
        }
    } else if event.id == W2R_EVT_PEER_MESSAGE
        && let Some(m) = unsafe { event.payload.cast::<W2rEvtPeerMessage>().as_ref() }
    {
        let _ = std::panic::catch_unwind(|| crate::countdown::on_peer(m));
    }
}

/// The plugin's rows on the Plugins screen: the same choice as on the options screen, as a
/// dropdown (a slider whose caption names the choice on a host without card dropdowns).
unsafe extern "C" fn on_ui(ui: *const W2rUi) {
    let Some(ui) = (unsafe { ui.as_ref() }) else {
        return;
    };
    let mut value = screen().index() as i32;
    unsafe {
        let changed = match combo_of(ui) {
            Some(combo) => {
                let labels: Vec<W2rStr> = model::CHOICES.iter().map(|l| s(l)).collect();
                combo(
                    ui.ctx,
                    s(model::PAUSED_SCREEN_LABEL),
                    labels.as_ptr(),
                    labels.len() as u32,
                    &mut value,
                )
            }
            None => {
                let caption = format!(
                    "{}: {}",
                    model::PAUSED_SCREEN_LABEL,
                    model::CHOICES[value as usize]
                );
                (ui.slider_int)(
                    ui.ctx,
                    s(&caption),
                    &mut value,
                    0,
                    model::CHOICES.len() as i32 - 1,
                )
            }
        };
        if changed != 0 {
            let picked = PausedScreen::from_index(value as i64);
            SCREEN.store(picked.index(), Ordering::Relaxed);
            store(model::PAUSED_SCREEN_KEY, picked.index());
            reconcile_box();
            tint_chosen();
            reapply_while_paused();
        }
        (ui.label)(
            ui.ctx,
            s("Default: the game's grey-out. Recolor: the map stays in color."),
        );
        (ui.label)(
            ui.ctx,
            s("Tint: the grey takes the color of the player who paused."),
        );
        // Recolor always shows the box: its checkbox shows ticked and greyed (where the host can
        // grey a row) and a click on it changes nothing.
        let forced = screen() == PausedScreen::Recolor;
        let can_grey = ui.struct_size >= W2R_UI_TEXT_EDIT_SIZE;
        let mut boxed = crate::pause_box::shown() as i32;
        if forced && can_grey {
            (ui.set_active)(ui.ctx, 0);
        }
        if (ui.checkbox)(ui.ctx, s(model::BOX_LABEL), &mut boxed) != 0 && !forced {
            crate::pause_box::set_show(boxed != 0);
            if let Some(h) = host() {
                (h.settings_set_i64)(h.token, s(model::BOX_KEY), boxed as i64);
            }
        }
        if forced && can_grey {
            (ui.set_active)(ui.ctx, 1);
        }
        let mut place = crate::pause_box::position() as i32;
        let moved = match combo_of(ui) {
            Some(combo) => {
                let labels: Vec<W2rStr> = model::BOX_POSITIONS.iter().map(|l| s(l)).collect();
                combo(
                    ui.ctx,
                    s(model::BOX_POSITION_LABEL),
                    labels.as_ptr(),
                    labels.len() as u32,
                    &mut place,
                )
            }
            None => {
                let caption = format!(
                    "{}: {}",
                    model::BOX_POSITION_LABEL,
                    model::BOX_POSITIONS[place as usize]
                );
                (ui.slider_int)(
                    ui.ctx,
                    s(&caption),
                    &mut place,
                    0,
                    model::BOX_POSITIONS.len() as i32 - 1,
                )
            }
        };
        if moved != 0 {
            let index = model::box_position(place as i64);
            crate::pause_box::set_position(index);
            if let Some(h) = host() {
                (h.settings_set_i64)(h.token, s(model::BOX_POSITION_KEY), index as i64);
            }
        }
        (ui.label)(
            ui.ctx,
            s("Pause Box: who paused, and online their pauses used and left."),
        );
        let mut counting = crate::countdown::enabled() as i32;
        if (ui.checkbox)(ui.ctx, s(model::COUNTDOWN_LABEL), &mut counting) != 0 {
            crate::countdown::set_enabled(counting != 0);
            store(model::COUNTDOWN_KEY, counting as i64);
        }
        let mut volume = crate::countdown::volume();
        if (ui.slider_int)(ui.ctx, s(model::VOLUME_LABEL), &mut volume, 0, 100) != 0 {
            crate::countdown::set_volume(volume);
            store(model::VOLUME_KEY, volume as i64);
        }
        (ui.label)(
            ui.ctx,
            s("Resume Countdown: a Resume waits 5 seconds, then resumes for everyone."),
        );
        (ui.label)(
            ui.ctx,
            s("Recolor always shows it. All of these are also under Options, UI, Pause Pro UI."),
        );
        (ui.label)(
            ui.ctx,
            s("Countdown beeps from Countdown Sound by Beau Hastings."),
        );
    }
}

fn remove_hook() {
    let hook = HOOK.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let (Some(h), Some(hook)) = (host(), hook) {
        unsafe { (h.hook_remove)(h.token, hook) };
    }
}

/// Puts the mode-2 slot back as the game had it (the tint program is left to leak: the game may
/// still be drawing with it this frame).
fn restore_slot() {
    if let Some(t) = TINT.lock().unwrap_or_else(|e| e.into_inner()).take()
        && unsafe { *(t.slot as *const usize) } == t.program
    {
        unsafe { *(t.slot as *mut usize) = t.previous };
    }
}

unsafe extern "C" fn on_load(host: *const W2rHost) -> W2rStatus {
    HOST.store(host.cast_mut(), Ordering::Release);
    let Some(h) = self::host() else {
        return W2R_E_STATE;
    };
    let declared = declare() & crate::pause_box::declare(h) & crate::countdown::declare(h);
    load_screen();
    crate::pause_box::load_setting();
    crate::countdown::load_setting();
    // Recolor chosen in an earlier session: the stored box setting is ticked for the options screen.
    reconcile_box();
    unsafe { (h.subscribe)(h.token, W2R_EVT_SETTINGS_CHANGED) };
    unsafe { (h.subscribe)(h.token, W2R_EVT_PEER_MESSAGE) };
    // SetPostEffect is a newer host symbol: an older host does not know it.
    let Some(target) = handle_of("SetPostEffect") else {
        log(
            W2R_LOG_ERROR,
            "symbol SetPostEffect not available (host too old?); the grey-out stays",
        );
        return W2R_E_STATE;
    };
    let mut trampoline: *mut c_void = core::ptr::null_mut();
    let mut hook = 0;
    let status = unsafe {
        (h.hook_install)(
            h.token,
            target,
            set_post_effect_detour as *mut c_void,
            &mut trampoline,
            &mut hook,
        )
    };
    if status != W2R_OK {
        log(
            W2R_LOG_ERROR,
            &format!("SetPostEffect: hook_install returned {status}; the grey-out stays"),
        );
        return W2R_E_STATE;
    }
    ORIGINAL.store(trampoline as usize, Ordering::Release);
    *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    // Tint's symbols; its program only when Tint is chosen (see `tint_chosen`).
    let tint = match GAME.get_or_init(resolve) {
        Ok(_) if screen() != PausedScreen::Tint => "Tint available",
        Ok(_) if tint_chosen() => "Tint ready",
        Ok(_) => "Tint program built at the first Tint pause",
        Err(name) => {
            log(
                W2R_LOG_WARN,
                &format!("symbol {name} not available (host too old?); no Tint"),
            );
            "no Tint"
        }
    };
    let pause_box = match crate::pause_box::install(h) {
        Ok(()) => format!(
            "pause box {}",
            if crate::pause_box::show() {
                "on"
            } else {
                "off"
            }
        ),
        Err(why) => {
            log(W2R_LOG_WARN, &format!("{why}; no pause box"));
            "no pause box".to_string()
        }
    };
    // The countdown runs from the box's frame hook: without it a Resume stays as it was.
    let countdown = if pause_box.starts_with("no ") {
        "no resume countdown"
    } else {
        match crate::countdown::install(h) {
            Ok(()) if crate::countdown::enabled() => "resume countdown on",
            Ok(()) => "resume countdown off",
            Err(why) => {
                log(W2R_LOG_WARN, &format!("{why}; no resume countdown"));
                "no resume countdown"
            }
        }
    };
    log(
        W2R_LOG_INFO,
        &format!(
            "Pause Pro ready; paused screen {}; {tint}; {pause_box}; {countdown}{}",
            model::CHOICES[screen().index() as usize],
            if declared {
                ""
            } else {
                "; not on the options screen (older host)"
            }
        ),
    );
    W2R_OK
}

unsafe extern "C" fn on_unload() {
    crate::countdown::remove();
    crate::pause_box::remove();
    remove_hook();
    restore_slot();
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
