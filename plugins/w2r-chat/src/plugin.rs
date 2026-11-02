//! The game-facing half of the chat plugin (32-bit only): symbol resolution, the detours, and
//! the Nuklear drawing, ported from wc2r-plus `chat.cpp` with its constants and offsets (game
//! build 1.0.2.2818). Everything here runs on the game's main thread except possibly the message
//! hooks, so state sits behind a mutex that is never held across a call into the game.
//!
//! Detours are outside the host's crash guard (docs/abi.md): each one forwards to the original
//! whatever happens in our code, and a panic is caught before it can unwind into the game.

use core::ffi::{CStr, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use w2r_abi::*;

mod hud;

use crate::model::{
    self, Backlog, CURSOR_COLOUR, Channel, Message, NameSpan, Palette, Settings, TEXT_GREY,
    apply_colour_factor, faded, revealed, rgba, visibility,
};

// ---- Nuklear and game layout (wc2r-plus target/struct_offsets.h) ---------------------------

mod off {
    pub const CTX_CURRENT_WINDOW: usize = 0x3d28;
    pub const CTX_FONT: usize = 0x1e0;
    pub const USER_FONT_HEIGHT: usize = 0x04;
    pub const USER_FONT_WIDTH: usize = 0x08;
    pub const CTX_TEXT_EXTRA0: usize = 0x210;
    pub const CTX_TEXT_PADDING_X: usize = 0x214;
    pub const CTX_TEXT_PADDING_Y: usize = 0x218;
    pub const CTX_TEXT_EXTRA1: usize = 0x21c;
    pub const CTX_TEXT_EXTRA2: usize = 0x220;
    pub const CTX_TEXT_COLOUR: usize = 0x20c;
    pub const CTX_TEXT_COLOUR_FACTOR: usize = 0x224;
    pub const CTX_WINDOW_BACKGROUND: usize = 0x1f9c;
    pub const CTX_SCROLLBAR_SIZE: usize = 0x2008;
    pub const CTX_GROUP_PADDING_Y: usize = 0x201c;
    pub const CTX_SCROLL_H: usize = 0x13d0;
    pub const CTX_SCROLL_V: usize = 0x1648;
    pub const WINDOW_BUFFER: usize = 0x64;
    pub const WINDOW_PANEL: usize = 0x8c;
    pub const PANEL_BOUNDS_X: usize = 0x08;
    pub const PANEL_OFFSET_Y: usize = 0x1c;
    pub const PANEL_AT_Y: usize = 0x24;
    pub const PANEL_ROW_HEIGHT: usize = 0x6c;
    pub const SCROLLBAR_STRIDE: usize = 0x278;
    pub const SCROLLBAR_TROUGH: [usize; 3] = [0x00, 0x20, 0x40];
    pub const SCROLLBAR_CURSOR: [usize; 3] = [0x64, 0x84, 0xa4];
    pub const SCROLLBAR_BORDER: usize = 0xc8;
    pub const SCROLLBAR_BORDER_CURSOR: usize = 0xd0;
    pub const SCROLLBAR_SHOW_BUTTONS: usize = 0xe8;
    pub const MESSAGE_SLOT_STRIDE: usize = 0xd0;
    pub const MESSAGE_SLOT_HIGHLIGHT: usize = 0xcc;
    pub const CTX_MOUSE_BUTTONS: usize = 0x17c;
    pub const MOUSE_BUTTON_STRIDE: usize = 0x10;
    pub const MOUSE_BUTTON_DOWN: usize = 0x00;
    pub const MOUSE_BUTTON_CLICKED: usize = 0x04;
    pub const MOUSE_LEFT: usize = 0;
    pub const MOUSE_RIGHT: usize = 2;
}

const SLOT_STICKY: usize = 15;
const SLOT_PROMPT: usize = 16;
const SCROLL_STYLE_SPAN: usize = off::SCROLLBAR_STRIDE * 2;
const SCROLLV_OFFSET: usize = off::CTX_SCROLL_V - off::CTX_SCROLL_H;
const SCROLL_TO_BOTTOM_Y: i32 = 1_000_000;
const INDICATOR_WIDTH: f32 = 4.0;
/// The game's player count: a sender index at or above this is not a player.
const PLAYER_COUNT: u8 = 8;
/// Key event layout and the two scan codes channels care about (wc2r-plus `struct_offsets.h`).
const EVENT_NUMBER: usize = 0x00;
const EVENT_SCAN: usize = 0x0c;
const EVENT_MODIFIERS: usize = 0x0d;
const SCAN_ENTER: u8 = 0x1c;
const SCAN_TAB: u8 = 0x0f;
/// Bytes per player in the game's name table.
const NAME_STRIDE: usize = 0x38;
/// Bytes per lobby slot, and the offset of its team byte (wc2r-plus `struct_offsets.h`).
const LOBBY_SLOT_STRIDE: usize = 0x26;
const LOBBY_SLOT_TEAM: usize = 0x0a;
/// Bytes per player in the game's diplomacy table.
const DIPLOMACY_ROW_STRIDE: usize = 0x10;

unsafe fn rd<T: Copy>(base: *const u8, offset: usize) -> T {
    unsafe { base.add(offset).cast::<T>().read_unaligned() }
}

unsafe fn wr<T: Copy>(base: *mut u8, offset: usize, value: T) {
    unsafe { base.add(offset).cast::<T>().write_unaligned(value) }
}

unsafe fn current_window(ctx: *const u8) -> *const u8 {
    unsafe { rd::<*const u8>(ctx, off::CTX_CURRENT_WINDOW) }
}

unsafe fn current_panel(ctx: *const u8) -> *const u8 {
    let win = unsafe { current_window(ctx) };
    if win.is_null() {
        core::ptr::null()
    } else {
        unsafe { rd::<*const u8>(win, off::WINDOW_PANEL) }
    }
}

unsafe fn window_canvas(ctx: *const u8) -> *mut c_void {
    let win = unsafe { current_window(ctx) };
    if win.is_null() {
        core::ptr::null_mut()
    } else {
        unsafe { win.add(off::WINDOW_BUFFER) as *mut c_void }
    }
}

unsafe fn scroll_offset_y(panel: *const u8) -> f32 {
    let p = unsafe { rd::<*const u32>(panel, off::PANEL_OFFSET_Y) };
    if p.is_null() {
        0.0
    } else {
        unsafe { *p as f32 }
    }
}

// ---- resolved game functions ---------------------------------------------------------------

type DrawMapMessageLineFn =
    unsafe extern "C" fn(*mut c_void, *const c_char, f32, f32, f32, f32, i32, i32, i32) -> i32;
type CountWrappedLinesFn = unsafe extern "C" fn(*mut c_void, *const c_char, i32, f32) -> i32;
type OverrideNextRectFn = unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32);
type DrawWrappedLabelFn = unsafe extern "C" fn(*mut c_void, *const c_char, u32);
type ContentRegionSizeFn = unsafe extern "C" fn(*mut c_void) -> u64;
type BeginNamedGroupFn = unsafe extern "C" fn(*mut c_void, *const c_char, u32) -> i32;
type LayoutRowDynamicFn = unsafe extern "C" fn(*mut c_void, f32, i32);
type ScrollGroupToFn = unsafe extern "C" fn(*mut c_void, *const c_char, i32, i32);
type LayoutSpaceBeginFn = unsafe extern "C" fn(*mut c_void, i32, f32, i32);
type CtxFn = unsafe extern "C" fn(*mut c_void);
type ExpireSlotFn = unsafe extern "C" fn(*mut c_void, u32);
type GetClockMsFn = unsafe extern "C" fn() -> u32;
type FillRectFn = unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, f32, u32);
type AllocSpaceFn = unsafe extern "C" fn(*mut f32, *mut c_void);
type TextClampFn = unsafe extern "C" fn(
    *const c_void,
    *const c_char,
    i32,
    f32,
    *mut i32,
    *mut f32,
    *const u32,
    i32,
) -> i32;
type WidgetTextFn = unsafe extern "C" fn(
    *mut c_void,
    f32,
    f32,
    f32,
    f32,
    *const c_char,
    i32,
    *const c_void,
    u32,
    *const c_void,
);
type TextWidthFn = unsafe extern "C" fn(*mut c_void, f32, *const c_char, i32) -> f32;

type PushMapMessageFn = unsafe extern "C" fn(*const c_char, i32, u32);
type DrawMapMessageListFn = unsafe extern "C" fn(*mut c_void, i32);
type BuildMapMessagesPanelFn = unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, i32);
type DisplayChatOrGameMessageFn = unsafe extern "C" fn(*const c_char, u8, u32, c_char);
type WriteStickyMessageFn = unsafe extern "C" fn(*const c_char, u32);
type PauseCommandFn = unsafe extern "C" fn();
type ToggleChatComposeFn = unsafe extern "C" fn(*mut c_void, *mut u8, c_char);
type KeyProcFn = unsafe extern "C" fn(*mut c_void, *mut u8) -> u32;
type SendComposedChatFn = unsafe extern "C" fn(*const c_char);
type WriteChatPromptLineFn = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char);

struct Game {
    draw_message_line: DrawMapMessageLineFn,
    count_wrapped: CountWrappedLinesFn,
    override_rect: OverrideNextRectFn,
    draw_wrapped_label: DrawWrappedLabelFn,
    content_region: ContentRegionSizeFn,
    begin_group: BeginNamedGroupFn,
    row_dynamic: LayoutRowDynamicFn,
    scroll_group_to: ScrollGroupToFn,
    space_begin: LayoutSpaceBeginFn,
    space_end: CtxFn,
    end_group: CtxFn,
    expire_slot: ExpireSlotFn,
    get_clock_ms: GetClockMsFn,
    fill_rect: FillRectFn,
    alloc_space: AllocSpaceFn,
    text_clamp: TextClampFn,
    widget_text: WidgetTextFn,
    wrap_separators: *const u32,
    chat_target_mode: *mut u8,
    chat_recipient_mask: *mut u8,
    chat_dialog: *const *mut c_void,
    toggle_compose: ToggleChatComposeFn,
    pause_flags: *const u8,
    slots: *mut u8,
    player_names: *const u8,
    command_issuer: *const u8,
    pauses_remaining: *const u8,
    networked_match: *const u8,
    // slices 3 and 4: who is who
    colour_reorder: *const u8,
    colour_rgba: *const f32,
    player_owner: *const u8,
    diplomacy: *const u8,
    lobby_slots: *const u8,
    local_player: *const u8,
}

// SAFETY: plain function and data addresses of the game, fixed for the session.
unsafe impl Send for Game {}
unsafe impl Sync for Game {}

static GAME: OnceLock<Game> = OnceLock::new();

fn game() -> Option<&'static Game> {
    GAME.get()
}

/// A name colour, with the black player's taken from the game's colour table so a recolour
/// (w2r-black-recolor) shows in chat; the stock table value keeps the readable grey.
fn name_colour_now(index: u8, fallback: u32) -> u32 {
    if index == model::BLACK
        && let Some(g) = game()
    {
        let c = unsafe { g.colour_rgba.add(index as usize * 4) };
        let rgb = [unsafe { *c }, unsafe { *c.add(1) }, unsafe { *c.add(2) }];
        return model::black_name(rgb);
    }
    model::name_colour(index, fallback)
}

// ---- host access ---------------------------------------------------------------------------

static HOST: AtomicPtr<W2rHost> = AtomicPtr::new(core::ptr::null_mut());

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

fn lookup(name: &str) -> Option<usize> {
    let h = host()?;
    let mut handle = 0;
    let mut addr = 0usize;
    unsafe {
        if (h.symbol_lookup)(h.token, s(name), &mut handle) != W2R_OK {
            return None;
        }
        if (h.symbol_address)(h.token, handle, &mut addr) != W2R_OK {
            return None;
        }
    }
    Some(addr)
}

fn handle_of(name: &str) -> Option<W2rHandle> {
    let h = host()?;
    let mut handle = 0;
    (unsafe { (h.symbol_lookup)(h.token, s(name), &mut handle) } == W2R_OK).then_some(handle)
}

macro_rules! need {
    ($name:literal, $t:ty) => {{
        let addr = lookup($name).ok_or($name)?;
        unsafe { core::mem::transmute::<usize, $t>(addr) }
    }};
}

fn resolve() -> Result<Game, &'static str> {
    Ok(Game {
        draw_message_line: need!("DrawMapMessageLine", DrawMapMessageLineFn),
        count_wrapped: need!("CountWrappedLines", CountWrappedLinesFn),
        override_rect: need!("OverrideNextRect", OverrideNextRectFn),
        draw_wrapped_label: need!("DrawWrappedLabel", DrawWrappedLabelFn),
        content_region: need!("ContentRegionSize", ContentRegionSizeFn),
        begin_group: need!("BeginNamedGroup", BeginNamedGroupFn),
        row_dynamic: need!("LayoutRowDynamic", LayoutRowDynamicFn),
        scroll_group_to: need!("ScrollGroupTo", ScrollGroupToFn),
        space_begin: need!("LayoutSpaceBegin", LayoutSpaceBeginFn),
        space_end: need!("LayoutSpaceEnd", CtxFn),
        end_group: need!("EndGroup", CtxFn),
        expire_slot: need!("ExpireSlot", ExpireSlotFn),
        get_clock_ms: need!("GetClockMs", GetClockMsFn),
        fill_rect: need!("NkFillRect", FillRectFn),
        alloc_space: need!("NkPanelAllocSpace", AllocSpaceFn),
        text_clamp: need!("NkTextClamp", TextClampFn),
        widget_text: need!("NkWidgetText", WidgetTextFn),
        wrap_separators: lookup("NkWrapSeparators").ok_or("NkWrapSeparators")? as *const u32,
        chat_target_mode: lookup("ChatTargetMode").ok_or("ChatTargetMode")? as *mut u8,
        chat_dialog: lookup("ChatDialog").ok_or("ChatDialog")? as *const *mut c_void,
        toggle_compose: need!("ToggleChatCompose", ToggleChatComposeFn),
        chat_recipient_mask: lookup("ChatRecipientMask").ok_or("ChatRecipientMask")? as *mut u8,
        pause_flags: lookup("MatchPauseFlags").ok_or("MatchPauseFlags")? as *const u8,
        slots: lookup("MapMessageSlots").ok_or("MapMessageSlots")? as *mut u8,
        player_names: lookup("PlayerNames").ok_or("PlayerNames")? as *const u8,
        command_issuer: lookup("CommandIssuer").ok_or("CommandIssuer")? as *const u8,
        pauses_remaining: lookup("PausesRemaining").ok_or("PausesRemaining")? as *const u8,
        networked_match: lookup("NetworkedMatch").ok_or("NetworkedMatch")? as *const u8,
        colour_reorder: lookup("TeamColourReorder").ok_or("TeamColourReorder")? as *const u8,
        colour_rgba: lookup("TeamColourRgba").ok_or("TeamColourRgba")? as *const f32,
        player_owner: lookup("PlayerOwner").ok_or("PlayerOwner")? as *const u8,
        diplomacy: lookup("DiplomacyStance").ok_or("DiplomacyStance")? as *const u8,
        lobby_slots: lookup("MpLobbySlots").ok_or("MpLobbySlots")? as *const u8,
        local_player: lookup("LocalPlayer").ok_or("LocalPlayer")? as *const u8,
    })
}

// ---- state ---------------------------------------------------------------------------------

#[derive(Default)]
struct State {
    settings: Settings,
    backlog: Backlog,
    /// The player whose chat line the game is displaying right now (set around
    /// `DisplayChatOrGameMessage`, read by the nested `PushMapMessage`).
    sender: Option<Sender>,
    log: LogFile,
    /// The channel being composed in (`System` when not composing) and the channel the prompt
    /// line was last written in.
    compose: Channel,
    prompt_channel: Channel,
    /// The channel composed in last (`System` for none yet); plain Enter reopens in it. Kept
    /// across matches.
    last_channel: Channel,
    /// The teammates (same lobby team and allied) taken at match start, as a recipient mask.
    team_mask: u8,
    /// The text of the three channel colour boxes in the plugin screen (NUL-terminated); the
    /// game edits a copy each frame and what it typed is stored back.
    hex_edit: [[u8; HEX_EDIT_CAP]; 3],
}

/// Room for six hex digits and a NUL, with a little slack for a longer paste.
const HEX_EDIT_CAP: usize = 16;

/// The chat line the game is displaying: who sent it, the raw text and the target mode.
#[derive(Clone, Default)]
struct Sender {
    who: u8,
    raw: Option<Vec<u8>>,
    target_mode: u8,
}

/// The per-launch disk log (`fs.data`). One file per launch, opened on the first line written
/// while the setting is on.
#[derive(Default)]
struct LogFile {
    handle: Option<W2rHandle>,
    name: Option<String>,
    /// Opening or writing failed; one warning was logged and the log stays off until the
    /// setting is toggled again.
    dead: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Locks the state, creating it on first use; a poisoned lock is still usable.
fn state() -> MutexGuard<'static, Option<State>> {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(State::default());
    }
    g
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = state();
    f(g.as_mut().expect("created"))
}

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// The colours of this frame's chat lines: the settings, or the game's own text colour when
/// channels are off (`model::Palette`).
unsafe fn palette(ctx: *mut c_void) -> Palette {
    let game_text = if ctx.is_null() {
        TEXT_GREY
    } else {
        unsafe { rd::<u32>(ctx.cast(), off::CTX_TEXT_COLOUR) }
    };
    with_state(|st| Palette::new(&st.settings, game_text))
}

fn enabled() -> bool {
    with_state(|st| st.settings.enabled)
}

fn channels_on() -> bool {
    with_state(|st| st.settings.enabled && st.settings.channels)
}

fn target_mode(g: &Game) -> u8 {
    unsafe { *g.chat_target_mode }
}

/// Recipient mask of the other in-match players allied to the local player, optionally only those
/// on the local player's lobby team too (wc2r-plus `AlliedMask`, `TeammatesAtMatchStart`).
fn recipients(g: &Game, same_lobby_team: bool) -> u8 {
    let me = unsafe { *g.local_player } as usize;
    if me >= PLAYER_COUNT as usize {
        return 0;
    }
    let mut in_match = [false; model::PLAYER_COUNT];
    let mut allied = [false; model::PLAYER_COUNT];
    let mut team = [false; model::PLAYER_COUNT];
    let lobby_team =
        |i: usize| unsafe { *g.lobby_slots.add(i * LOBBY_SLOT_STRIDE + LOBBY_SLOT_TEAM) };
    for i in 0..model::PLAYER_COUNT {
        unsafe {
            in_match[i] = *g.player_owner.add(i) <= 1;
            allied[i] = *g.diplomacy.add(me * DIPLOMACY_ROW_STRIDE + i) != 0;
        }
        team[i] = lobby_team(i) == lobby_team(me);
    }
    model::recipient_mask(me, &in_match, &allied, same_lobby_team.then_some(&team))
}

fn allied_mask(g: &Game) -> u8 {
    recipients(g, false)
}

// ---- original functions (trampolines) ------------------------------------------------------

static ORIG_PUSH: AtomicUsize = AtomicUsize::new(0);
static ORIG_DRAW_LIST: AtomicUsize = AtomicUsize::new(0);
static ORIG_BUILD_PANEL: AtomicUsize = AtomicUsize::new(0);
static ORIG_DISPLAY: AtomicUsize = AtomicUsize::new(0);
static ORIG_STICKY: AtomicUsize = AtomicUsize::new(0);
static ORIG_PAUSE: AtomicUsize = AtomicUsize::new(0);
static ORIG_RESUME: AtomicUsize = AtomicUsize::new(0);
static ORIG_TEXTBOX_KEY: AtomicUsize = AtomicUsize::new(0);
static ORIG_MINIMAP_KEY: AtomicUsize = AtomicUsize::new(0);
static ORIG_SEND_CHAT: AtomicUsize = AtomicUsize::new(0);
static ORIG_PROMPT: AtomicUsize = AtomicUsize::new(0);
/// True while the game's own pause or resume handler runs: its sticky message is ours to
/// replace with an announcement.
static IN_PAUSE_HANDLER: AtomicBool = AtomicBool::new(false);

macro_rules! original {
    ($slot:expr, $t:ty) => {{
        #[allow(unused_unsafe)]
        unsafe {
            core::mem::transmute::<usize, $t>($slot.load(Ordering::Acquire))
        }
    }};
}

// ---- drawing -------------------------------------------------------------------------------

/// The scroll area of the current group, as the game reports it (`AbsoluteGroupRegion`).
unsafe fn group_region(g: &Game, ctx: *mut c_void) -> Option<(f32, f32)> {
    let packed = unsafe { (g.content_region)(ctx) };
    let w = f32::from_bits((packed & 0xffff_ffff) as u32);
    let h = f32::from_bits((packed >> 32) as u32);
    let padding = unsafe { rd::<f32>(ctx as *const u8, off::CTX_GROUP_PADDING_Y) };
    let h = h - padding * 2.0;
    (w > 8.0 && w < 20000.0 && h > 0.0 && h < 20000.0).then_some((w, h))
}

unsafe fn fill_group_background(g: &Game, ctx: *mut c_void, alpha: i32, max_h: f32) {
    if alpha <= 0 {
        return;
    }
    let c = ctx as *const u8;
    let canvas = unsafe { window_canvas(c) };
    let panel = unsafe { current_panel(c) };
    if canvas.is_null() || panel.is_null() {
        return;
    }
    let b = unsafe { rd::<[f32; 4]>(panel, off::PANEL_BOUNDS_X) };
    if b[2] > 0.0 && b[3] > 0.0 {
        unsafe {
            (g.fill_rect)(
                canvas,
                b[0],
                b[1],
                b[2],
                b[3].min(max_h),
                0.0,
                rgba(0, 0, 0, alpha as u8),
            )
        };
    }
}

unsafe fn flatten_scrollbar(sb: *mut u8, trough: u32, cursor: u32) {
    unsafe {
        for o in off::SCROLLBAR_TROUGH {
            wr::<u32>(sb, o, 0);
            wr::<u32>(sb, o + 4, trough);
        }
        for o in off::SCROLLBAR_CURSOR {
            wr::<u32>(sb, o, 0);
            wr::<u32>(sb, o + 4, cursor);
        }
        wr::<f32>(sb, off::SCROLLBAR_BORDER, 0.0);
        wr::<f32>(sb, off::SCROLLBAR_BORDER_CURSOR, 0.0);
    }
}

unsafe fn draw_scroll_indicator(g: &Game, ctx: *mut c_void, strip_w: f32) {
    let c = ctx as *const u8;
    let canvas = unsafe { window_canvas(c) };
    let panel = unsafe { current_panel(c) };
    if canvas.is_null() || panel.is_null() {
        return;
    }
    let bounds = unsafe { rd::<[f32; 4]>(panel, off::PANEL_BOUNDS_X) };
    let at_y = unsafe { rd::<f32>(panel, off::PANEL_AT_Y) };
    let row_h = unsafe { rd::<f32>(panel, off::PANEL_ROW_HEIGHT) };
    let offset_y = unsafe { scroll_offset_y(panel) };

    let content_h = (at_y + row_h) - bounds[1];
    let view_h = bounds[3];
    if content_h <= view_h + 0.5 {
        return;
    }
    let max_offset = content_h - view_h;
    let thumb_h = (view_h * (view_h / content_h)).max(strip_w).min(view_h);
    let frac = if max_offset > 0.0 {
        (offset_y / max_offset).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_y = bounds[1] + frac * (view_h - thumb_h);
    let thumb_x = bounds[0] + bounds[2] - strip_w;
    unsafe {
        (g.fill_rect)(
            canvas,
            thumb_x,
            thumb_y,
            strip_w,
            thumb_h,
            strip_w * 0.5,
            CURSOR_COLOUR,
        )
    };
}

struct LineLook {
    /// Draw the colour spans of `Message::names`.
    names: bool,
    alpha: f32,
    band: u32,
}

/// One message laid out by hand so it can fade and sit on a band: wraps it with the game's own
/// text clamp, and draws each line with the game's own widget text (`DrawNamedLine`; slice 4 adds
/// the coloured name spans).
unsafe fn draw_line(g: &Game, ctx: *mut c_void, m: &Message, colour: u32, look: &LineLook) {
    let c = ctx as *const u8;
    if unsafe { current_panel(c) }.is_null() {
        return;
    }
    let canvas = unsafe { window_canvas(c) };
    let font = unsafe { rd::<*const u8>(c, off::CTX_FONT) };
    if canvas.is_null() || font.is_null() {
        return;
    }
    let font_userdata = unsafe { rd::<*mut c_void>(font, 0) };
    let font_h = unsafe { rd::<f32>(font, off::USER_FONT_HEIGHT) };
    let Some(width) = (unsafe { rd::<Option<TextWidthFn>>(font, off::USER_FONT_WIDTH) }) else {
        return;
    };

    let mut b = [0f32; 4];
    unsafe { (g.alloc_space)(b.as_mut_ptr(), ctx) };
    if look.band >> 24 != 0 {
        unsafe { (g.fill_rect)(canvas, b[0], b[1], b[2], b[3], 0.0, look.band) };
    }

    let pad_x = unsafe { rd::<f32>(c, off::CTX_TEXT_PADDING_X) };
    let pad_y = unsafe { rd::<f32>(c, off::CTX_TEXT_PADDING_Y) };
    let factor = unsafe { rd::<f32>(c, off::CTX_TEXT_COLOUR_FACTOR) };

    let mut style = [0u32; 7];
    unsafe {
        style[2] = rd::<u32>(c, off::CTX_WINDOW_BACKGROUND);
        style[4] = rd::<u32>(c, off::CTX_TEXT_EXTRA0);
        style[5] = rd::<u32>(c, off::CTX_TEXT_EXTRA1);
        style[6] = rd::<u32>(c, off::CTX_TEXT_EXTRA2);
    }
    let text_fg = faded(apply_colour_factor(colour, factor), look.alpha);

    let box_w = b[2].max(pad_x * 2.0);
    let box_h = b[3].max(pad_y * 2.0);
    let bottom = b[1] + (box_h - pad_y * 2.0);
    let line_x = b[0] + pad_x;
    let line_w = box_w - pad_x * 2.0;
    let line_h = pad_y * 2.0 + font_h;
    let mut line_y = b[1] + pad_y;

    let text = m.text.as_ptr();
    let len = m.text.as_bytes().len() as i32;
    let mut done = 0i32;
    let mut glyphs = 0i32;
    let mut fit_w = 0f32;
    let clamp = |from: i32, glyphs: &mut i32, fit_w: &mut f32| unsafe {
        (g.text_clamp)(
            font.cast(),
            text.add(from as usize),
            len - from,
            line_w,
            glyphs,
            fit_w,
            g.wrap_separators,
            1,
        )
    };
    let mut fitting = clamp(done, &mut glyphs, &mut fit_w);
    while done < len && fitting != 0 && line_y + line_h <= bottom {
        let line_end = done + fitting;
        // A piece of this wrapped line, drawn after the width of what precedes it.
        let piece = |from: i32, to: i32, fg: u32, style: &mut [u32; 7]| {
            if to <= from {
                return;
            }
            let dx = if from > done {
                unsafe { width(font_userdata, font_h, text.add(done as usize), from - done) }
            } else {
                0.0
            };
            style[3] = fg;
            unsafe {
                (g.widget_text)(
                    canvas,
                    line_x + dx,
                    line_y,
                    line_w - dx,
                    line_h,
                    text.add(from as usize),
                    to - from,
                    style.as_ptr().cast(),
                    0x11,
                    font.cast(),
                );
            }
        };
        let mut at = done;
        if look.names {
            for n in &m.names {
                let from = (n.start as i32).clamp(done, line_end);
                let to = ((n.start + n.len) as i32).clamp(done, line_end);
                if to <= from {
                    continue;
                }
                piece(at, from, text_fg, &mut style);
                let name_fg = faded(
                    apply_colour_factor(name_colour_now(n.colour, colour), factor),
                    look.alpha,
                );
                piece(from, to, name_fg, &mut style);
                at = to;
            }
        }
        piece(at, line_end, text_fg, &mut style);
        done = line_end;
        line_y += pad_y * 2.0 + font_h;
        fitting = clamp(done, &mut glyphs, &mut fit_w);
    }
}

unsafe fn wrapped_lines(g: &Game, ctx: *mut c_void, m: &Message, w: f32) -> i32 {
    let n = unsafe { (g.count_wrapped)(ctx, m.text.as_ptr(), m.text.as_bytes().len() as i32, w) };
    n.max(1)
}

unsafe fn draw_backlog_rows(g: &Game, ctx: *mut c_void, w: f32, line_height: f32) {
    let (messages, name_colours) =
        with_state(|st| (st.backlog.snapshot(), st.settings.name_colours));
    let palette = unsafe { palette(ctx) };
    for m in &messages {
        let n = unsafe { wrapped_lines(g, ctx, m, w) };
        unsafe {
            (g.row_dynamic)(ctx, n as f32 * line_height, 1);
            if name_colours && !m.names.is_empty() {
                draw_line(
                    g,
                    ctx,
                    m,
                    m.colour(&palette),
                    &LineLook {
                        names: true,
                        alpha: 1.0,
                        band: 0,
                    },
                );
            } else {
                (g.draw_wrapped_label)(ctx, m.text.as_ptr(), m.colour(&palette));
            }
        }
    }
    unsafe { (g.row_dynamic)(ctx, 0.01, 1) };
    if with_state(|st| st.backlog.take_scroll()) {
        unsafe { (g.scroll_group_to)(ctx, c"messages_history".as_ptr(), 0, SCROLL_TO_BOTTOM_Y) };
    }
}

unsafe fn draw_recent_lines(g: &Game, ctx: *mut c_void, line_height: i32) {
    let Some((w, h)) = (unsafe { group_region(g, ctx) }) else {
        return;
    };
    let (hold_ms, bg, name_colours, recent) = with_state(|st| {
        (
            st.settings.fade_seconds.max(0) as u64 * 1000,
            st.settings.bg_alpha,
            st.settings.name_colours,
            // Enough to fill the biggest panel; the loop stops at the first one that does not fit.
            st.backlog
                .recent_first()
                .take(32)
                .cloned()
                .collect::<Vec<_>>(),
        )
    });
    let palette = unsafe { palette(ctx) };
    let now = now_ms();
    let max_lines = h as i32 / line_height;
    let mut used = 0;
    for m in &recent {
        let alpha = visibility(m.arrived_ms, now, hold_ms);
        if alpha <= 0.0 {
            break;
        }
        let n = unsafe { wrapped_lines(g, ctx, m, w) };
        if used + n > max_lines {
            break;
        }
        used += n;
        unsafe {
            (g.override_rect)(
                ctx,
                0.0,
                h - (used * line_height) as f32,
                w,
                (n * line_height) as f32,
            );
            draw_line(
                g,
                ctx,
                m,
                m.colour(&palette),
                &LineLook {
                    names: name_colours,
                    alpha,
                    band: rgba(0, 0, 0, (bg as f32 * alpha) as u8),
                },
            );
        }
    }
}

// ---- capture and the disk log --------------------------------------------------------------

fn local_time() -> model::LocalTime {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut t) };
    model::LocalTime {
        year: t.wYear,
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
    }
}

/// Opens the log on first use. `None` when logging is off or has failed.
fn log_handle(time: &model::LocalTime) -> Option<W2rHandle> {
    let (wanted, existing, dead) = with_state(|st| {
        (
            st.settings.enabled && st.settings.log,
            st.log.handle,
            st.log.dead,
        )
    });
    if !wanted || dead {
        return None;
    }
    if existing.is_some() {
        return existing;
    }
    // Named from the moment logging first turns on (wc2r-plus `SyncChatLog`), not from the first
    // message of the session.
    let name = with_state(|st| {
        st.log
            .name
            .get_or_insert_with(|| model::log_file_name(time))
            .clone()
    });
    let h = host()?;
    let mut handle = 0;
    let status = unsafe { (h.data_open)(h.token, s(&name), W2R_FILE_WRITE, &mut handle) };
    if status != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("could not open the chat log {name} (status {status}); logging is off"),
        );
        with_state(|st| st.log.dead = true);
        return None;
    }
    with_state(|st| st.log.handle = Some(handle));
    Some(handle)
}

fn log_write(handle: W2rHandle, text: &str) {
    let Some(h) = host() else { return };
    let status =
        unsafe { (h.data_write)(h.token, handle, text.as_ptr().cast(), text.len() as u32) };
    if status != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("chat log write failed (status {status}); logging is off"),
        );
        with_state(|st| st.log.dead = true);
    }
}

/// Closes the log; the next line (if the setting is on) opens a new file name only on a new
/// launch, so re-enabling appends nothing to a closed file but starts it afresh.
fn close_log() {
    let handle = with_state(|st| {
        st.log.dead = false;
        st.log.name = None;
        st.log.handle.take()
    });
    if let (Some(handle), Some(h)) = (handle, host()) {
        unsafe { (h.data_close)(h.token, handle) };
    }
}

/// A player's name bytes from the game's table, `None` for an empty or unterminated slot.
fn player_name_bytes(g: &Game, who: usize) -> Option<Vec<u8>> {
    if who >= PLAYER_COUNT as usize {
        return None;
    }
    let slot =
        unsafe { core::slice::from_raw_parts(g.player_names.add(who * NAME_STRIDE), NAME_STRIDE) };
    model::name_bytes(slot)
}

/// The player's colour index, 8 (none) when the game has not assigned one.
fn colour_index(g: &Game, who: usize) -> u8 {
    if who >= PLAYER_COUNT as usize {
        return model::PLAYER_COUNT as u8;
    }
    let index = unsafe { *g.colour_reorder.add(who) };
    if (index as usize) < model::PLAYER_COUNT {
        index
    } else {
        model::PLAYER_COUNT as u8
    }
}

/// Human players with a real name and colour, for spotting names inside system lines.
fn human_players(g: &Game) -> Vec<(Vec<u8>, u8)> {
    (0..PLAYER_COUNT as usize)
        .filter_map(|i| {
            let name = player_name_bytes(g, i)?;
            let colour = colour_index(g, i);
            (unsafe { *g.player_owner.add(i) } == 0
                && name.len() >= 2
                && (colour as usize) < model::PLAYER_COUNT)
                .then_some((name, colour))
        })
        .collect()
}

/// One message into the backlog and the disk log, with the optional `[HH:MM] ` prefix
/// (wc2r-plus `CaptureMessage`).
fn capture(text: &[u8], highlight: bool, channel: Channel, names: Vec<NameSpan>) {
    let (stamps, cap) = with_state(|st| (st.settings.timestamps, st.settings.backlog));
    let time = local_time();
    let mut bytes = Vec::with_capacity(text.len() + 8);
    if stamps {
        bytes.extend_from_slice(model::timestamp_prefix(&time).as_bytes());
    }
    let shift = bytes.len();
    bytes.extend_from_slice(text);
    let names = names
        .into_iter()
        .map(|n| NameSpan {
            start: n.start + shift,
            ..n
        })
        .collect();
    let Some(m) = Message::new(&bytes, highlight, channel, now_ms()).map(|m| m.with_names(names))
    else {
        return;
    };
    if let Some(handle) = log_handle(&time) {
        log_write(handle, &model::log_line(&time, &m.text.to_string_lossy()));
    }
    with_state(|st| st.backlog.push(m, cap));
}

// ---- detours -------------------------------------------------------------------------------

/// The colour span of the sender's name in a chat line, if the line has one and the game gave
/// the player a colour.
fn find_sender_span(line: &[u8], name: &[u8], colour: u8) -> Vec<NameSpan> {
    match model::find_sender_name(line, name) {
        Some(start) if (colour as usize) < model::PLAYER_COUNT => vec![NameSpan {
            start,
            len: name.len(),
            colour,
        }],
        _ => Vec::new(),
    }
}

unsafe extern "C" fn push_map_message(text: *const c_char, highlight: i32, duration_ms: u32) {
    if !text.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if !enabled() {
                return;
            }
            let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
            let Some(g) = game() else { return };
            let (sender, channels) = with_state(|st| {
                (
                    st.sender.clone(),
                    st.settings.enabled && st.settings.channels,
                )
            });
            // A line the game is displaying for a player is chat (the sender's name in their
            // colour, labelled with its channel); anything else is a system line (gold) with
            // every player name it mentions coloured (wc2r-plus `HookedPushMapMessage`).
            let sender_name = sender
                .as_ref()
                .and_then(|s| player_name_bytes(g, s.who as usize));
            if let (Some(sender), Some(name), false) = (&sender, &sender_name, channels) {
                let colour = colour_index(g, sender.who as usize);
                let names = find_sender_span(bytes, name, colour);
                capture(bytes, highlight != 0, Channel::All, names);
            } else if let (Some(sender), Some(name), Some(raw)) = (
                &sender,
                &sender_name,
                sender.as_ref().and_then(|s| s.raw.as_ref()),
            ) {
                let channel = model::channel_of(sender.target_mode);
                let mut line = channel.label().as_bytes().to_vec();
                line.push(b' ');
                let colour = colour_index(g, sender.who as usize);
                let names = if (colour as usize) < model::PLAYER_COUNT {
                    vec![NameSpan {
                        start: line.len(),
                        len: name.len(),
                        colour,
                    }]
                } else {
                    Vec::new()
                };
                line.extend_from_slice(name);
                line.extend_from_slice(b": ");
                line.extend_from_slice(raw);
                capture(&line, highlight != 0, channel, names);
            } else {
                let mut line = bytes.to_vec();
                let names = model::find_player_names(&line, &human_players(g));
                model::square_whisper_brackets(&mut line, &names);
                capture(&line, highlight != 0, Channel::System, names);
            }
        }));
    }
    unsafe { original!(ORIG_PUSH, PushMapMessageFn)(text, highlight, duration_ms) };
}

unsafe extern "C" fn display_chat_or_game_message(
    text: *const c_char,
    who: u8,
    duration_ms: u32,
    target_mode: c_char,
) {
    let raw = (!text.is_null()).then(|| unsafe { CStr::from_ptr(text) }.to_bytes().to_vec());
    let saved = with_state(|st| {
        let saved = st.sender.take();
        st.sender = (who < PLAYER_COUNT).then_some(Sender {
            who,
            raw,
            target_mode: target_mode as u8,
        });
        saved
    });
    unsafe {
        original!(ORIG_DISPLAY, DisplayChatOrGameMessageFn)(text, who, duration_ms, target_mode)
    };
    with_state(|st| st.sender = saved);
}

/// While the game's own pause or resume handler runs, its sticky "paused" message is dropped:
/// the plugin announces the pause as a chat line instead.
unsafe extern "C" fn write_sticky_message(text: *const c_char, duration_ms: u32) {
    if IN_PAUSE_HANDLER.load(Ordering::Relaxed) && enabled() {
        return;
    }
    unsafe { original!(ORIG_STICKY, WriteStickyMessageFn)(text, duration_ms) };
}

fn announce_pause(g: &Game, paused: bool) {
    // Offline the game does not refill PlayerNames, so the issuer's slot may still hold a name
    // from the last online match; the game's own handler shows no name there either.
    if unsafe { *g.networked_match } == 0 {
        capture(
            model::pause_line(None, paused, None).as_bytes(),
            true,
            Channel::System,
            Vec::new(),
        );
        return;
    }
    let who = unsafe { *g.command_issuer };
    if who >= PLAYER_COUNT {
        return;
    }
    let slot = unsafe {
        core::slice::from_raw_parts(g.player_names.add(who as usize * NAME_STRIDE), NAME_STRIDE)
    };
    let Some(name) = model::player_name(slot) else {
        return;
    };
    let remaining = Some(unsafe { *g.pauses_remaining.add(who as usize) });
    let colour = colour_index(g, who as usize);
    let names = if (colour as usize) < model::PLAYER_COUNT {
        vec![NameSpan {
            start: 0,
            len: name.len(),
            colour,
        }]
    } else {
        Vec::new()
    };
    capture(
        model::pause_line(Some(&name), paused, remaining).as_bytes(),
        true,
        Channel::System,
        names,
    );
}

/// Runs the game's pause (or resume) handler with the sticky message muted, then announces the
/// change if the pause flag actually flipped.
unsafe fn pause_or_resume(original: &AtomicUsize, pausing: bool) {
    let handler = original!(original, PauseCommandFn);
    let flag = |g: &Game| unsafe { *g.pause_flags } & 1 != 0;
    let before = game().map(flag).unwrap_or(false);
    IN_PAUSE_HANDLER.store(true, Ordering::Relaxed);
    unsafe { handler() };
    IN_PAUSE_HANDLER.store(false, Ordering::Relaxed);
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(g) = game() else { return };
        if !enabled() {
            return;
        }
        let after = flag(g);
        if (pausing && !before && after) || (!pausing && before && !after) {
            announce_pause(g, pausing);
        }
    }));
}

unsafe extern "C" fn handle_pause_command() {
    unsafe { pause_or_resume(&ORIG_PAUSE, true) };
}

unsafe extern "C" fn handle_resume_command() {
    unsafe { pause_or_resume(&ORIG_RESUME, false) };
}

// ---- channels (slice 3) --------------------------------------------------------------------

fn key_event(event: *mut u8) -> (u16, u8, u8) {
    unsafe {
        (
            rd::<u16>(event, EVENT_NUMBER),
            rd::<u8>(event, EVENT_SCAN),
            rd::<u8>(event, EVENT_MODIFIERS),
        )
    }
}

/// Composing in an enhanced channel: the box is open and a channel was chosen for it.
fn composing(g: &Game) -> bool {
    let (on, compose) = with_state(|st| (st.settings.enabled && st.settings.channels, st.compose));
    on && target_mode(g) != 0 && compose != Channel::System
}

fn compose_on(g: &Game, channel: Channel) {
    with_state(|st| {
        st.compose = channel;
        st.last_channel = channel;
    });
    unsafe { *g.chat_target_mode = model::compose_mode_of(channel) };
}

fn on_compose_opened(g: &Game, modifiers: u8) {
    let mode = target_mode(g);
    if mode == 0 || mode == 1 {
        with_state(|st| st.compose = Channel::System);
        return;
    }
    let has_team = with_state(|st| st.team_mask) != 0;
    let last = with_state(|st| st.last_channel);
    let channel = model::open_channel(modifiers, has_team, allied_mask(g) != 0, last);
    compose_on(g, channel);
}

fn cycle_compose_channel(g: &Game) {
    let (current, team_mask) = with_state(|st| (st.compose, st.team_mask));
    let next = model::next_channel(current, team_mask != 0, allied_mask(g) != 0);
    compose_on(g, next);
    log(
        W2R_LOG_INFO,
        &format!("compose channel -> {}", next.label()),
    );
}

unsafe extern "C" fn chat_textbox_event_proc(dlg: *mut c_void, event: *mut u8) -> u32 {
    let original = original!(ORIG_TEXTBOX_KEY, KeyProcFn);
    let Some(g) = game().filter(|_| !event.is_null() && enabled()) else {
        return unsafe { original(dlg, event) };
    };
    let (number, scan, modifiers) = key_event(event);
    if catch_unwind(AssertUnwindSafe(|| composing(g))).unwrap_or(false)
        && scan == SCAN_TAB
        && number == 1
    {
        return 1;
    }
    let opening = target_mode(g) == 0 && number == 0 && scan == SCAN_ENTER;
    let handled = unsafe { original(dlg, event) };
    if opening && channels_on() {
        let _ = catch_unwind(AssertUnwindSafe(|| on_compose_opened(g, modifiers)));
    }
    handled
}

unsafe extern "C" fn minimap_terrain_key_proc(dlg: *mut c_void, event: *mut u8) -> u32 {
    let original = original!(ORIG_MINIMAP_KEY, KeyProcFn);
    if let Some(g) = game().filter(|_| !event.is_null() && enabled()) {
        let (number, scan, _) = key_event(event);
        let handled = catch_unwind(AssertUnwindSafe(|| {
            if composing(g) && scan == SCAN_TAB && number == 0 {
                cycle_compose_channel(g);
                true
            } else {
                false
            }
        }))
        .unwrap_or(false);
        if handled {
            return 1;
        }
    }
    unsafe { original(dlg, event) }
}

unsafe extern "C" fn send_composed_chat(text: *const c_char) {
    let original = original!(ORIG_SEND_CHAT, SendComposedChatFn);
    let Some(g) = game().filter(|_| enabled()) else {
        return unsafe { original(text) };
    };
    if !catch_unwind(AssertUnwindSafe(|| composing(g))).unwrap_or(false) {
        return unsafe { original(text) };
    }
    let (compose, team_mask) = with_state(|st| (st.compose, st.team_mask));
    unsafe {
        if compose == Channel::Allies {
            if allied_mask(g) == 0 {
                capture(
                    b"You have no allies and your message was not sent.",
                    true,
                    Channel::System,
                    Vec::new(),
                );
                return;
            }
            *g.chat_target_mode = 3;
            original(text);
        } else if compose == Channel::Team && team_mask != 0 {
            let saved = *g.chat_recipient_mask;
            *g.chat_recipient_mask = team_mask;
            *g.chat_target_mode = model::TEAM_TARGET_MODE;
            original(text);
            *g.chat_recipient_mask = saved;
        } else {
            *g.chat_target_mode = 2;
            original(text);
        }
    }
}

unsafe extern "C" fn write_chat_prompt_line(
    format: *const c_char,
    arg1: *const c_char,
    arg2: *const c_char,
) {
    let original = original!(ORIG_PROMPT, WriteChatPromptLineFn);
    let Some(g) = game() else {
        return unsafe { original(format, arg1, arg2) };
    };
    if target_mode(g) == 0 {
        with_state(|st| st.compose = Channel::System);
    }
    let channel = if format.is_null() || !enabled() || target_mode(g) == 5 {
        None
    } else {
        catch_unwind(AssertUnwindSafe(|| composing(g)))
            .unwrap_or(false)
            .then(|| with_state(|st| st.compose))
    };
    let Some(channel) = channel else {
        with_state(|st| st.prompt_channel = Channel::System);
        return unsafe { original(format, arg1, arg2) };
    };
    let mut line = channel.label().as_bytes().to_vec();
    line.push(b' ');
    if !arg1.is_null() {
        line.extend_from_slice(unsafe { CStr::from_ptr(arg1) }.to_bytes());
    }
    line.push(0);
    with_state(|st| st.prompt_channel = channel);
    unsafe { original(c"%s".as_ptr(), line.as_ptr().cast(), core::ptr::null()) };
}

/// True when the mouse button was pressed this frame (wc2r-plus `nk::MousePressed`).
unsafe fn mouse_pressed(ctx: *const u8, button: usize) -> bool {
    let b = off::CTX_MOUSE_BUTTONS + button * off::MOUSE_BUTTON_STRIDE;
    unsafe {
        rd::<i32>(ctx, b + off::MOUSE_BUTTON_DOWN) != 0
            && rd::<u32>(ctx, b + off::MOUSE_BUTTON_CLICKED) != 0
    }
}

/// "Click closes chat": a left or right click while composing discards the draft and returns the
/// keyboard to the game (wc2r-plus `CloseComposeOnClick`).
unsafe fn close_compose_on_click(g: &Game, ctx: *mut c_void) {
    if !with_state(|st| st.settings.click_unfocus) || target_mode(g) == 0 {
        return;
    }
    let ctx = ctx as *const u8;
    let left = unsafe { mouse_pressed(ctx, off::MOUSE_LEFT) };
    if !left && !unsafe { mouse_pressed(ctx, off::MOUSE_RIGHT) } {
        return;
    }
    let dialog = unsafe { *g.chat_dialog };
    if dialog.is_null() {
        return;
    }
    let mut unread = [0u8; 16];
    unsafe { (g.toggle_compose)(dialog, unread.as_mut_ptr(), 0) };
    log(
        W2R_LOG_INFO,
        &format!(
            "compose closed by a {} click",
            if left { "left" } else { "right" }
        ),
    );
}

/// One prompt line in the channel's colour; returns the lines used so far (the game's
/// `DrawMapMessageLine` contract), `0x11` when it does not fit (wc2r-plus `DrawLineInColour`).
unsafe fn draw_line_in_colour(
    g: &Game,
    ctx: *mut c_void,
    text: *const c_char,
    (w, h): (f32, f32),
    colour: u32,
    used: i32,
    line_height: i32,
) -> i32 {
    let len = unsafe { CStr::from_ptr(text) }.to_bytes().len() as i32;
    let n = unsafe { (g.count_wrapped)(ctx, text, len, w) };
    let total = used + n.max(1);
    if total >= 0x12 {
        return 0x11;
    }
    unsafe {
        (g.override_rect)(
            ctx,
            0.0,
            h - (total * line_height) as f32,
            w,
            (n * line_height) as f32,
        );
        (g.draw_wrapped_label)(ctx, text, colour);
    }
    total
}

unsafe extern "C" fn draw_map_message_list(ctx: *mut c_void, line_height: i32) {
    let original = original!(ORIG_DRAW_LIST, DrawMapMessageListFn);
    let drawn = catch_unwind(AssertUnwindSafe(|| {
        let Some(g) = game() else { return false };
        if !enabled() || ctx.is_null() || line_height <= 0 {
            return false;
        }
        unsafe { draw_message_strip(g, ctx, line_height) }
    }))
    .unwrap_or(false);
    if !drawn {
        unsafe { original(ctx, line_height) };
    }
}

/// The prompt line, then the sticky line below it. False when the group has no usable region
/// (the game's own drawing runs instead).
unsafe fn draw_message_strip(g: &Game, ctx: *mut c_void, line_height: i32) -> bool {
    let Some((w, h)) = (unsafe { group_region(g, ctx) }) else {
        return false;
    };
    unsafe {
        let prompt = g.slots.add(SLOT_PROMPT * off::MESSAGE_SLOT_STRIDE);
        let sticky = g.slots.add(SLOT_STICKY * off::MESSAGE_SLOT_STRIDE);
        let prompt_channel = with_state(|st| st.prompt_channel);
        let used = if prompt_channel != Channel::System {
            draw_line_in_colour(
                g,
                ctx,
                prompt.cast(),
                (w, h),
                palette(ctx).of(prompt_channel),
                0,
                line_height,
            )
        } else {
            (g.draw_message_line)(
                ctx,
                prompt.cast(),
                0.0,
                0.0,
                w,
                h,
                rd::<u8>(prompt, off::MESSAGE_SLOT_HIGHLIGHT) as i32,
                0,
                line_height,
            )
        };
        (g.draw_message_line)(
            ctx,
            sticky.cast(),
            0.0,
            0.0,
            w,
            h,
            rd::<u8>(sticky, off::MESSAGE_SLOT_HIGHLIGHT) as i32,
            used,
            line_height,
        );
    }
    true
}

unsafe extern "C" fn build_map_messages_panel(
    ctx: *mut c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    line_height: i32,
) {
    let original = original!(ORIG_BUILD_PANEL, BuildMapMessagesPanelFn);
    let Some(g) = game() else {
        return unsafe { original(ctx, x, y, w, h, line_height) };
    };
    if !enabled() || ctx.is_null() || line_height <= 0 {
        return unsafe { original(ctx, x, y, w, h, line_height) };
    }
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        close_compose_on_click(g, ctx)
    }));

    // The game's style fields are borrowed for the duration and always put back, even if our
    // drawing panics.
    let c = ctx as *mut u8;
    let scroll_style = unsafe { c.add(off::CTX_SCROLL_H) };
    let mut saved_style = [0u8; SCROLL_STYLE_SPAN];
    unsafe {
        core::ptr::copy_nonoverlapping(scroll_style, saved_style.as_mut_ptr(), SCROLL_STYLE_SPAN)
    };
    let saved_scrollbar = unsafe { rd::<[f32; 2]>(c, off::CTX_SCROLLBAR_SIZE) };
    let drew = catch_unwind(AssertUnwindSafe(|| unsafe {
        draw_panel(g, ctx, x, y, w, h, line_height, scroll_style)
    }));
    unsafe {
        core::ptr::copy_nonoverlapping(saved_style.as_ptr(), scroll_style, SCROLL_STYLE_SPAN);
        wr::<[f32; 2]>(c, off::CTX_SCROLLBAR_SIZE, saved_scrollbar);
    }
    if drew.is_err() {
        // Nothing sensible was drawn: let the game draw its own.
        unsafe { original(ctx, x, y, w, h, line_height) };
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_panel(
    g: &Game,
    ctx: *mut c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    line_height: i32,
    scroll_style: *mut u8,
) {
    let lh = line_height as f32;
    let now = unsafe { (g.get_clock_ms)() };
    for i in 0..15 {
        unsafe { (g.expire_slot)(g.slots.add(i * off::MESSAGE_SLOT_STRIDE).cast(), now) };
    }
    unsafe {
        (g.expire_slot)(
            g.slots.add(SLOT_STICKY * off::MESSAGE_SLOT_STRIDE).cast(),
            now,
        )
    };

    let prompt = unsafe { *g.slots.add(SLOT_PROMPT * off::MESSAGE_SLOT_STRIDE) };
    let sticky = unsafe { *g.slots.add(SLOT_STICKY * off::MESSAGE_SLOT_STRIDE) };
    let strip_has_content = prompt != 0 || sticky != 0;
    let strip_h = (2.0 * lh).min(h);

    let (lines, bg, fade) = with_state(|st| {
        (
            st.settings.lines,
            st.settings.bg_alpha,
            st.settings.fade_seconds,
        )
    });
    let history_max = h - strip_h;
    let history_h = (lines as f32 * lh).min(history_max).max(0.0);
    let panel_h = history_h + strip_h;
    // Anchored to the bottom, as wc2r-plus does.
    let top = y + h - panel_h;

    unsafe {
        flatten_scrollbar(scroll_style.add(SCROLLV_OFFSET), 0, 0);
        wr::<i32>(
            scroll_style,
            SCROLLV_OFFSET + off::SCROLLBAR_SHOW_BUTTONS,
            0,
        );
        wr::<[f32; 2]>(
            ctx as *mut u8,
            off::CTX_SCROLLBAR_SIZE,
            [INDICATOR_WIDTH, 0.0],
        );
    }

    static WAS_REVEALED: Mutex<bool> = Mutex::new(true);
    let is_revealed = unsafe { revealed(fade, *g.chat_target_mode, *g.pause_flags) };
    {
        let mut was = WAS_REVEALED.lock().unwrap_or_else(|e| e.into_inner());
        if is_revealed && !*was {
            with_state(|st| st.backlog.request_scroll());
        }
        *was = is_revealed;
    }

    if !is_revealed && history_h > lh * 0.5 {
        unsafe {
            (g.override_rect)(ctx, x, top, w, history_h);
            if (g.begin_group)(ctx, c"messages_recent".as_ptr(), 0x20) != 0 {
                (g.space_begin)(ctx, 1, 0.0, 1000);
                draw_recent_lines(g, ctx, line_height);
                (g.space_end)(ctx);
                (g.end_group)(ctx);
            }
        }
    }

    if is_revealed && history_h > lh * 0.5 {
        unsafe {
            (g.override_rect)(ctx, x, top, w, history_h);
            if (g.begin_group)(ctx, c"messages_history".as_ptr(), 0) != 0 {
                fill_group_background(g, ctx, bg, f32::MAX);
                draw_backlog_rows(g, ctx, w, lh);
                draw_scroll_indicator(g, ctx, INDICATOR_WIDTH);
                (g.end_group)(ctx);
            }
        }
    }

    if strip_has_content {
        unsafe {
            (g.override_rect)(ctx, x, top + history_h, w, strip_h);
            if (g.begin_group)(ctx, c"messages".as_ptr(), 0x20) != 0 {
                (g.space_begin)(ctx, 1, 0.0, 1000);
                if !draw_message_strip(g, ctx, line_height) {
                    // No region to draw into this frame; nothing to do.
                }
                (g.space_end)(ctx);
                (g.end_group)(ctx);
            }
        }
    }
}

// ---- hooks ---------------------------------------------------------------------------------

static HOOKS: Mutex<Vec<W2rHandle>> = Mutex::new(Vec::new());

fn install_hook(
    symbol: &str,
    detour: *mut c_void,
    original: &AtomicUsize,
) -> Result<W2rHandle, String> {
    let h = host().ok_or("no host")?;
    let target = handle_of(symbol).ok_or_else(|| format!("{symbol}: unknown symbol"))?;
    let mut trampoline: *mut c_void = core::ptr::null_mut();
    let mut hook = 0;
    let status = unsafe { (h.hook_install)(h.token, target, detour, &mut trampoline, &mut hook) };
    if status != W2R_OK {
        return Err(format!("{symbol}: hook_install returned {status}"));
    }
    original.store(trampoline as usize, Ordering::Release);
    Ok(hook)
}

fn remove_hooks() {
    let handles = std::mem::take(&mut *HOOKS.lock().unwrap_or_else(|e| e.into_inner()));
    if let Some(h) = host() {
        for hook in handles.into_iter().rev() {
            unsafe { (h.hook_remove)(h.token, hook) };
        }
    }
}

// ---- plugin entry --------------------------------------------------------------------------

fn load_settings() -> Settings {
    let mut settings = Settings::default();
    if let Some(h) = host() {
        for key in [
            model::ENABLED.key,
            model::LINES.key,
            model::FADE.key,
            model::BACKLOG.key,
            model::BG.key,
            model::TIMESTAMPS.key,
            model::LOG.key,
            model::NAME_COLOURS.key,
            model::CHANNELS.key,
            model::CLICK_UNFOCUS.key,
        ] {
            let mut v = 0i64;
            if unsafe { (h.settings_get_i64)(h.token, s(key), &mut v) } == W2R_OK {
                settings.apply(key, v);
            }
        }
    }
    settings
}

/// A stored string setting, if the host has one.
fn load_str(key: &str) -> Option<String> {
    let h = host()?;
    let mut buf = [0u8; 64];
    let mut len = 0u32;
    let status =
        unsafe { (h.settings_get_str)(h.token, s(key), buf.as_mut_ptr().cast(), 64, &mut len) };
    if status != W2R_OK {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..(len as usize).min(buf.len())]).into_owned())
}

/// Reads the channel colour settings into the state and the edit boxes. A value that is not
/// six hex digits is logged once and the channel keeps its default (wc2r-plus
/// `WarnBadChannelColours`).
fn load_colours() {
    for (i, spec) in model::CHANNEL_COLOURS.iter().enumerate() {
        let text = load_str(spec.key).unwrap_or_else(|| spec.default_hex.to_owned());
        let valid = with_state(|st| {
            let valid = st.settings.apply_hex(spec.key, &text);
            let n = text.len().min(HEX_EDIT_CAP - 1);
            st.hex_edit[i] = [0; HEX_EDIT_CAP];
            st.hex_edit[i][..n].copy_from_slice(&text.as_bytes()[..n]);
            valid
        });
        if !valid {
            log(
                W2R_LOG_WARN,
                &format!(
                    "{} \"{text}\" is not six hex digits (e.g. {}) -- using {}",
                    spec.key, spec.default_hex, spec.default_hex
                ),
            );
        }
    }
}

fn save_str(key: &str, value: &str) {
    if let Some(h) = host() {
        unsafe { (h.settings_set_str)(h.token, s(key), s(value)) };
    }
}

fn save_setting(key: &str, value: i64) {
    if let Some(h) = host() {
        unsafe { (h.settings_set_i64)(h.token, s(key), value) };
    }
}

unsafe extern "C" fn on_load(host: *const W2rHost) -> W2rStatus {
    HOST.store(host.cast_mut(), Ordering::Release);
    let g = match resolve() {
        Ok(g) => g,
        Err(name) => {
            log(
                W2R_LOG_ERROR,
                &format!("symbol {name} not available; chat not started"),
            );
            return W2R_E_STATE;
        }
    };
    let _ = GAME.set(g);
    with_state(|st| st.settings = load_settings());
    load_colours();

    let hooks: [(&str, *mut c_void, &AtomicUsize); 11] = [
        (
            "PushMapMessage",
            push_map_message as *mut c_void,
            &ORIG_PUSH,
        ),
        (
            "DisplayChatOrGameMessage",
            display_chat_or_game_message as *mut c_void,
            &ORIG_DISPLAY,
        ),
        (
            "DrawMapMessageList",
            draw_map_message_list as *mut c_void,
            &ORIG_DRAW_LIST,
        ),
        (
            "BuildMapMessagesPanel",
            build_map_messages_panel as *mut c_void,
            &ORIG_BUILD_PANEL,
        ),
        (
            "WriteStickyMessage",
            write_sticky_message as *mut c_void,
            &ORIG_STICKY,
        ),
        (
            "HandlePauseCommand",
            handle_pause_command as *mut c_void,
            &ORIG_PAUSE,
        ),
        (
            "HandleResumeCommand",
            handle_resume_command as *mut c_void,
            &ORIG_RESUME,
        ),
        (
            "ChatTextboxEventProc",
            chat_textbox_event_proc as *mut c_void,
            &ORIG_TEXTBOX_KEY,
        ),
        (
            "MinimapTerrainKeyProc",
            minimap_terrain_key_proc as *mut c_void,
            &ORIG_MINIMAP_KEY,
        ),
        (
            "SendComposedChat",
            send_composed_chat as *mut c_void,
            &ORIG_SEND_CHAT,
        ),
        (
            "WriteChatPromptLine",
            write_chat_prompt_line as *mut c_void,
            &ORIG_PROMPT,
        ),
    ];
    let mut hooks = hooks.to_vec();
    // The paused-game HUD (wc2r-plus in_match_hud.cpp). Optional: without it the history still
    // shows while typing, just not while paused.
    static HUD_BUILD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    match hud::resolve() {
        Ok(()) => {
            hooks.push((
                "BuildInMatchHud",
                hud::build_in_match_hud as *mut c_void,
                &HUD_BUILD_ORIGINAL,
            ));
            hooks.push((
                "EditControlEventProc",
                hud::edit_control_event as *mut c_void,
                &hud::ORIG_EDIT_PROC,
            ));
        }
        Err(name) => log(
            W2R_LOG_WARN,
            &format!("symbol {name} not available; the HUD is left alone (no chat while paused)"),
        ),
    }
    for (name, detour, original) in hooks {
        match install_hook(name, detour, original) {
            Ok(handle) => HOOKS.lock().unwrap_or_else(|e| e.into_inner()).push(handle),
            Err(msg) => {
                log(W2R_LOG_ERROR, &msg);
                remove_hooks();
                return W2R_E_STATE;
            }
        }
    }
    if let Some(h) = self::host() {
        unsafe { (h.subscribe)(h.token, W2R_EVT_MATCH_START) };
    }
    log(W2R_LOG_INFO, "chat history active");
    W2R_OK
}

unsafe extern "C" fn on_event(event: *const W2rEvent) {
    let id = unsafe { (*event).id };
    if id == W2R_EVT_MATCH_START {
        let mask = game().map_or(0, |g| recipients(g, true));
        with_state(|st| {
            st.backlog.clear();
            st.team_mask = mask;
            st.compose = Channel::System;
            st.prompt_channel = Channel::System;
        });
        let time = local_time();
        if let Some(handle) = log_handle(&time) {
            log_write(handle, model::LOG_MATCH_MARKER);
        }
        log(
            W2R_LOG_INFO,
            &format!("match start: backlog cleared, teammates mask 0x{mask:02x}"),
        );
    }
}

unsafe extern "C" fn on_ui(ui: *const W2rUi) {
    let ui = unsafe { &*ui };
    let mut changed: Vec<(&'static str, i64)> = Vec::new();
    let (
        mut lines,
        mut fade,
        mut backlog,
        mut bg,
        mut stamps,
        mut log_on,
        mut names_on,
        mut channels,
        mut click,
    ) = with_state(|st| {
        (
            st.settings.lines,
            st.settings.fade_seconds,
            st.settings.backlog as i32,
            st.settings.bg_alpha,
            st.settings.timestamps as i32,
            st.settings.log as i32,
            st.settings.name_colours as i32,
            st.settings.channels as i32,
            st.settings.click_unfocus as i32,
        )
    });
    unsafe {
        (ui.label)(ui.ctx, s("Chat history (port of wc2r-plus)"));
        if (ui.slider_int)(
            ui.ctx,
            s("History height (lines)"),
            &mut lines,
            model::LINES.min as i32,
            model::LINES.max as i32,
        ) != 0
        {
            changed.push((model::LINES.key, lines as i64));
        }
        if (ui.slider_int)(
            ui.ctx,
            s("Fade time (seconds, 0 = never)"),
            &mut fade,
            model::FADE.min as i32,
            model::FADE.max as i32,
        ) != 0
        {
            changed.push((model::FADE.key, fade as i64));
        }
        if (ui.slider_int)(
            ui.ctx,
            s("Saved message limit"),
            &mut backlog,
            model::BACKLOG.min as i32,
            model::BACKLOG.max as i32,
        ) != 0
        {
            changed.push((model::BACKLOG.key, backlog as i64));
        }
        if (ui.slider_int)(
            ui.ctx,
            s("Background opacity"),
            &mut bg,
            model::BG.min as i32,
            model::BG.max as i32,
        ) != 0
        {
            changed.push((model::BG.key, bg as i64));
        }
        if (ui.checkbox)(ui.ctx, s("Show timestamps"), &mut stamps) != 0 {
            changed.push((model::TIMESTAMPS.key, stamps as i64));
        }
        if (ui.checkbox)(ui.ctx, s("Color player names"), &mut names_on) != 0 {
            changed.push((model::NAME_COLOURS.key, names_on as i64));
        }
        if (ui.checkbox)(ui.ctx, s("Enhanced chat channels"), &mut channels) != 0 {
            changed.push((model::CHANNELS.key, channels as i64));
        }
        // The colour boxes need the text widget, which hosts before it do not have.
        if ui.struct_size >= W2R_UI_TEXT_EDIT_SIZE {
            (ui.set_active)(ui.ctx, channels);
            for (i, spec) in model::CHANNEL_COLOURS.iter().enumerate() {
                let mut buf = with_state(|st| st.hex_edit[i]);
                if (ui.text_edit)(ui.ctx, s(spec.label), buf.as_mut_ptr(), buf.len() as u32) != 0 {
                    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    with_state(|st| {
                        st.hex_edit[i] = buf;
                        st.settings.apply_hex(spec.key, &text);
                    });
                    save_str(spec.key, &text);
                }
            }
            (ui.set_active)(ui.ctx, 1);
        }
        if (ui.checkbox)(ui.ctx, s("Click closes chat"), &mut click) != 0 {
            changed.push((model::CLICK_UNFOCUS.key, click as i64));
        }
        if (ui.checkbox)(ui.ctx, s("Log chat to disk"), &mut log_on) != 0 {
            changed.push((model::LOG.key, log_on as i64));
        }
    }
    for (key, value) in changed {
        with_state(|st| st.settings.apply(key, value));
        save_setting(key, value);
        if key == model::LOG.key && value == 0 {
            close_log();
        }
    }
}

unsafe extern "C" fn on_unload() {
    close_log();
    remove_hooks();
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
