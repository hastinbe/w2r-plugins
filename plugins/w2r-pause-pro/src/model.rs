//! The pure half of Pause Pro.
//!
//! The game greys the screen with a full-screen post-effect shader. `SetPostEffect(mode)` switches
//! it: 0 off, 1 grey, 2 red. The pause handler passes 1, the resume handler and the end of a
//! paused match pass 0; nothing in the game passes 2. Recolor turns the 1 into a 0. Tint puts a
//! tinted grey program into the slot mode 2 draws with and turns the 1 into a 2; the tint is the
//! colour of the player who paused.

/// The grey-out mode the pause handler asks for.
pub const GREY: i32 = 1;
/// The effect switched off.
pub const OFF: i32 = 0;
/// The mode whose program slot Tint takes over (stock: a red-only grey no caller uses).
pub const TINT_MODE: i32 = 2;

/// Setting key for the paused screen choice (the index into `CHOICES`).
pub const PAUSED_SCREEN_KEY: &str = "paused_screen";
pub const PAUSED_SCREEN_LABEL: &str = "Pause Screen";
pub const PAUSED_SCREEN_DESCRIPTION: &str = "How the map looks while a match is paused. Default: the game's grey-out. Recolor: the map \
     stays in color (the Pause Box is always shown). Tint: the grey takes the color of the player \
     who paused. Only your screen changes.";
/// Where the settings sit on the options screen: the host's UI category, under their own heading
/// ("Pause Pro UI").
pub const CATEGORY: &str = "UI";
pub const SUBCATEGORY: &str = "Pause Pro UI";
/// Dropdown labels, in `PausedScreen` order.
pub const CHOICES: [&str; 3] = ["Default", "Recolor", "Tint"];

/// What the map looks like while paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PausedScreen {
    /// The game's own grey-out (the default).
    Default,
    /// No greyscale: the map stays in colour.
    Recolor,
    /// The grey, tinted with the colour of the player who paused.
    Tint,
}

impl PausedScreen {
    pub const DEFAULT: PausedScreen = PausedScreen::Default;

    /// From the stored dropdown index; anything unknown is the default.
    pub fn from_index(i: i64) -> PausedScreen {
        match i {
            1 => PausedScreen::Recolor,
            2 => PausedScreen::Tint,
            _ => PausedScreen::DEFAULT,
        }
    }

    pub fn index(self) -> i64 {
        match self {
            PausedScreen::Default => 0,
            PausedScreen::Recolor => 1,
            PausedScreen::Tint => 2,
        }
    }
}

/// The mode to pass on to the game when it asks for `requested`. `tint_ready`: the tinted
/// program sits in the mode-2 slot; without it Tint falls back to the stock grey.
pub fn effect_mode(requested: i32, screen: PausedScreen, tint_ready: bool) -> i32 {
    match screen {
        PausedScreen::Recolor if requested == GREY => OFF,
        PausedScreen::Tint if requested == GREY && tint_ready => TINT_MODE,
        _ => requested,
    }
}

/// The player whose colour tints the pause: online the player whose pause command is being
/// handled (`CommandIssuer`), offline the local player (only they can pause). `None` for an
/// out-of-range slot.
pub fn pauser(networked: bool, issuer: u8, local: u8) -> Option<u8> {
    let who = if networked { issuer } else { local };
    (who < 8).then_some(who)
}

/// The tint for a team colour (0..1 floats): scaled so its brightest channel is 1, so a dark
/// colour (black, blue) still tints instead of darkening the whole screen. Grey-ish colours stay
/// grey-ish; a colour with no brightness at all is white (no tint).
pub fn tint_colour(rgb: [f32; 3]) -> [f32; 3] {
    let c = rgb.map(|v| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let max = c[0].max(c[1]).max(c[2]);
    if max < 0.02 {
        return [1.0, 1.0, 1.0];
    }
    c.map(|v| v / max)
}

/// Setting key for the pause box (1 on, 0 off; on when unset).
pub const BOX_KEY: &str = "pause_box";
pub const BOX_LABEL: &str = "Pause Box";
pub const BOX_DESCRIPTION: &str = "While a match is paused, a box in the middle of the map in your race's dialog art: who paused \
     (their color and name) and, online, how many pauses they have used and have left.";
/// Pauses each player gets in an online match (the game sets every player's count to 3 at match
/// start, networked only; offline there is no limit).
pub const PAUSES_PER_PLAYER: u8 = 3;
pub const BOX_TITLE: &str = "PAUSED";
/// Offline the game's name table still holds the previous online match's names, so the box does
/// not trust it and names the local player like this.
pub const OFFLINE_NAME: &str = "You";

/// "Pauses: 1 used, 2 left" from a player's remaining count.
pub fn pauses_line(remaining: u8) -> String {
    let left = remaining.min(PAUSES_PER_PLAYER);
    format!("Pauses: {} used, {} left", PAUSES_PER_PLAYER - left, left)
}

/// The name shown for the pauser: offline always `OFFLINE_NAME`; online the name table entry up
/// to its NUL, or "Player N" when it is empty.
pub fn display_name(raw: &[u8], slot: u8, networked: bool) -> String {
    if !networked {
        return OFFLINE_NAME.to_string();
    }
    let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    if len == 0 {
        return format!("Player {}", slot as u32 + 1);
    }
    String::from_utf8_lossy(&raw[..len]).into_owned()
}

/// A colour in the game's byte order (R in the low byte), opaque, from 0..1 floats.
pub fn pack(rgb: [f32; 3]) -> u32 {
    let byte = |v: f32| {
        (if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        } * 255.0
            + 0.5) as u32
    };
    0xff00_0000 | byte(rgb[2]) << 16 | byte(rgb[1]) << 8 | byte(rgb[0])
}

/// Where everything in the pause box goes, in the dialog layer's (absolute) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxLayout {
    /// The window: x, y, w, h.
    pub rect: [f32; 4],
    /// Text rows: x, y, w, h (the title and the count centred in theirs).
    pub title: [f32; 4],
    pub name: [f32; 4],
    pub count: Option<[f32; 4]>,
    /// The swatch's colour square (its outline is one unit bigger all round).
    pub swatch: [f32; 4],
}

/// Setting key for where the box sits (the index into `BOX_POSITIONS`).
pub const BOX_POSITION_KEY: &str = "pause_box_position";
pub const BOX_POSITION_LABEL: &str = "Pause Box Position";
pub const BOX_POSITION_DESCRIPTION: &str = "Where the Pause Box sits: at a corner, an edge or the center of the map view, or over the \
     selection display (the panel below the minimap).";
/// Dropdown labels; `place_box` takes the index. The first nine are the map view's 3 x 3 grid, row
/// by row.
pub const BOX_POSITIONS: [&str; 10] = [
    "Top Left",
    "Top Center",
    "Top Right",
    "Middle Left",
    "Center (Default)",
    "Middle Right",
    "Bottom Left",
    "Bottom Center",
    "Bottom Right",
    "Over Selection Display",
];
pub const BOX_POSITION_DEFAULT: i64 = 4;
const OVER_SELECTION: usize = 9;

/// The stored position index, anything unknown being the default (Center).
pub fn box_position(stored: i64) -> usize {
    if (0..BOX_POSITIONS.len() as i64).contains(&stored) {
        stored as usize
    } else {
        BOX_POSITION_DEFAULT as usize
    }
}

/// The Pause Box row's condition on the options screen: active unless Pause Screen is Recolor,
/// shown ticked while it is (`key!=N:V`, the locked checkbox of a host that has it).
pub fn box_lock_condition() -> String {
    format!("{PAUSED_SCREEN_KEY}!={}:1", PausedScreen::Recolor.index())
}

/// Whether the box shows: when its setting is on, and always with Recolor (the map stays in colour,
/// so the box is the only sign of the pause).
pub fn box_shown(setting_on: bool, screen: PausedScreen) -> bool {
    setting_on || screen == PausedScreen::Recolor
}

/// The game's picture (the HUD fitted in the window: all of it in HD, between the black side bars
/// in classic), the map view and the selection display, in window pixels (the dialog layer's
/// space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudRects {
    pub screen: [f32; 4],
    pub view: [f32; 4],
    pub selection: [f32; 4],
}

/// Where the in-match HUD's parts are, in the HUD's own units: the side panel's width, the bar
/// at the top, bottom and right around the map, and the selection display ("portbar") below the
/// minimap as x, y, w, h.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudLayout {
    pub panel: f32,
    pub bar: f32,
    pub selection: [f32; 4],
}

/// The HD view (as w2r-chat builds the HUD, after the stock builder): a HUD as wide as the window.
pub const HD_LAYOUT: HudLayout = HudLayout {
    panel: 176.0,
    bar: 16.0,
    selection: [0.0, 160.0, 188.0, 176.0],
};

/// The classic view (F5): an 800 x 600 HUD fitted to the window's height, with black bars at the
/// sides; the selection display is the side panel's width.
pub const CLASSIC_LAYOUT: HudLayout = HudLayout {
    panel: 176.0,
    bar: 16.0,
    selection: [0.0, 160.0, 176.0, 172.0],
};

/// The layout for `GraphicsMode` (0 classic, 1 HD; 2 and 3 are mid-switch, drawn as HD).
pub fn layout_for(graphics_mode: u32) -> &'static HudLayout {
    if graphics_mode == 0 {
        &CLASSIC_LAYOUT
    } else {
        &HD_LAYOUT
    }
}

/// From the dialog layer's width in pixels (`GetUiSpaceWidth`), the window's height over its
/// width (`None` when unknown: the HUD's own), and the HUD's size in its own units (`HudUiWidth`,
/// `HudUiHeight`, which follow the graphics mode): the HUD is fitted inside the window and centred
/// (the classic view's black side bars), and `layout` placed in it. `None` for sizes that make no
/// sense.
pub fn hud_rects(
    space_w: f32,
    aspect: Option<f32>,
    hud_w: f32,
    hud_h: f32,
    layout: &HudLayout,
) -> Option<HudRects> {
    let ok = |v: f32| v.is_finite() && v > 0.0;
    let min_w = layout.panel + layout.bar;
    if !(ok(space_w) && ok(hud_w) && ok(hud_h)) || hud_w <= min_w || hud_h <= layout.bar * 2.0 {
        return None;
    }
    let space_h = space_w * aspect.filter(|a| ok(*a)).unwrap_or(hud_h / hud_w);
    let k = (space_w / hud_w).min(space_h / hud_h);
    let (ox, oy) = ((space_w - hud_w * k) / 2.0, (space_h - hud_h * k) / 2.0);
    let r = |x: f32, y: f32, w: f32, h: f32| [ox + x * k, oy + y * k, w * k, h * k];
    let [sx, sy, sw, sh] = layout.selection;
    Some(HudRects {
        screen: r(0.0, 0.0, hud_w, hud_h),
        view: r(
            layout.panel,
            layout.bar,
            hud_w - layout.panel - layout.bar,
            hud_h - layout.bar * 2.0,
        ),
        selection: r(sx, sy, sw, sh),
    })
}

/// The box's top-left corner for `position` (an index into `BOX_POSITIONS`), `margin` in from the
/// map view's edges, kept inside the window.
pub fn place_box(position: usize, rects: &HudRects, w: f32, h: f32, margin: f32) -> (f32, f32) {
    let [vx, vy, vw, vh] = rects.view;
    let (x, y) = if position == OVER_SELECTION {
        // The visible frame top-aligned on the selection display, `margin` below the minimap's
        // border: the window starts higher by the dialog skin's own top inset.
        let [cx, cy, cw, _] = rects.selection;
        (cx + (cw - w) / 2.0, cy + margin - frame_top_inset(margin))
    } else {
        let x = match position % 3 {
            0 => vx + margin,
            1 => vx + (vw - w) / 2.0,
            _ => vx + vw - w - margin,
        };
        let y = match position / 3 {
            0 => vy + margin,
            // The visible frame centred, not the window: the frame sits lower than the window.
            1 => vy + (vh - h) / 2.0 - frame_centre_drop(margin),
            _ => vy + vh - h - margin,
        };
        (x, y)
    };
    let [sx, sy, sw, sh] = rects.screen;
    let x = x.min(sx + sw - w).max(sx);
    let y = y.min(sy + sh - h).max(sy);
    (x.round(), y.round())
}

/// How far below the window's top the stock dialog skin starts drawing its frame: measured in game
/// at a 3840 x 2400 window (font height 80, margin 40) as about 154 pixels, so 1.9 font heights
/// (`margin` is half a font height).
fn frame_top_inset(margin: f32) -> f32 {
    (margin * 2.0 * 1.9).round()
}

/// How far the middle of the skin's visible frame is below the middle of the window: the frame
/// starts 1.9 font heights below the window's top and ends about 0.9 below its bottom (measured
/// in game at 3840 x 2400, font height 80: frame 1040 to 1580 for a window at 890, 620 tall), so
/// 1.4 font heights.
fn frame_centre_drop(margin: f32) -> f32 {
    (margin * 2.0 * 1.4).round()
}

/// The box's size from the font's height and the text widths: `(w, h)`.
pub fn box_size(font_h: f32, title_w: f32, name_w: f32, count_w: Option<f32>) -> (f32, f32) {
    let m = Metrics::new(font_h, name_w);
    let content_w = title_w
        .max(m.who_w)
        .max(count_w.unwrap_or(0.0))
        .max(m.font_h * 10.0);
    let rows = if count_w.is_some() { 3.0 } else { 2.0 };
    (
        (content_w + m.pad * 2.0).round(),
        (m.row * rows + m.row * 0.5 + m.pad * 2.0).round(),
    )
}

struct Metrics {
    font_h: f32,
    pad: f32,
    row: f32,
    swatch: f32,
    gap: f32,
    who_w: f32,
}

impl Metrics {
    fn new(font_h: f32, name_w: f32) -> Metrics {
        let font_h = if font_h.is_finite() && font_h > 1.0 {
            font_h
        } else {
            16.0
        };
        let swatch = (font_h * 0.8).round();
        let gap = (font_h * 0.5).round();
        Metrics {
            font_h,
            pad: font_h * 2.0,
            row: font_h * 1.5,
            swatch,
            gap,
            who_w: swatch + gap + name_w.max(0.0),
        }
    }
}

/// The box laid out with its top-left corner at `(x, y)` and the size `box_size` gives.
pub fn box_layout(
    x: f32,
    y: f32,
    font_h: f32,
    title_w: f32,
    name_w: f32,
    count_w: Option<f32>,
) -> BoxLayout {
    let (w, h) = box_size(font_h, title_w, name_w, count_w);
    let Metrics {
        pad,
        row,
        swatch,
        gap,
        who_w,
        ..
    } = Metrics::new(font_h, name_w);
    let cx = x + w / 2.0;
    let inner_x = x + pad;
    let inner_w = w - pad * 2.0;
    let title = [inner_x, y + pad, inner_w, row];
    let who_y = y + pad + row * 1.5;
    let who_x = (cx - who_w / 2.0).round();
    let name = [who_x + swatch + gap, who_y, name_w.max(0.0) + 1.0, row];
    let swatch_rect = [
        who_x,
        (who_y + (row - swatch) / 2.0).round(),
        swatch,
        swatch,
    ];
    let count = count_w.map(|_| [inner_x, who_y + row, inner_w, row]);
    BoxLayout {
        rect: [x, y, w, h],
        title,
        name,
        count,
        swatch: swatch_rect,
    }
}

// ---- the resume countdown -----------------------------------------------------------------

/// The resume countdown: the full 5 seconds, as the lobby's start countdown.
pub const COUNTDOWN_SECONDS: u32 = 5;
pub const COUNTDOWN_KEY: &str = "resume_countdown";
pub const COUNTDOWN_LABEL: &str = "Resume Countdown";
pub const COUNTDOWN_DESCRIPTION: &str = "A Resume waits 5 seconds, counting down in the Pause Box (or the \
     message area when the box is off) with the lobby countdown's sound, then resumes for everyone. \
     Players with this plugin see and hear it too. Pressing Resume again cancels it.";
pub const VOLUME_KEY: &str = "countdown_volume";
pub const VOLUME_LABEL: &str = "Countdown Volume";
pub const VOLUME_DESCRIPTION: &str = "How loud the countdown's beeps are while the game is in the \
     background, 0 to 100 (0 is silent). In the game it plays the lobby tick at your sound effects \
     volume.";
/// A countdown is dropped this long after it should have ended without the resume taking effect
/// (the starter left, or the resume was lost), so Resume works again.
pub const STALL_GRACE_MS: u64 = 3_000;
/// The frame tick must have run this recently for a countdown to start; otherwise a Resume acts
/// as normal (nothing would finish the countdown).
pub const FRAME_FRESH_MS: u64 = 1_000;

/// One resume countdown, timed from `start_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Countdown {
    pub start_ms: u64,
    pub seconds: u32,
    /// Started by the local player (who sends the resume when it ends).
    pub mine: bool,
    /// The last number announced (a tick), `seconds + 1` before the first.
    shown: u32,
    done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountdownStep {
    None,
    /// The number now showing (5, 4, ... 1): play the tick.
    Tick(u32),
    /// It reached zero: play the go sound; the starter sends the resume.
    Go,
}

impl Countdown {
    pub fn new(start_ms: u64, seconds: u32, mine: bool) -> Countdown {
        let seconds = seconds.clamp(1, 10);
        Countdown {
            start_ms,
            seconds,
            mine,
            shown: seconds + 1,
            done: false,
        }
    }

    /// The number to show: `seconds` down to 1, then 0 when it has run out.
    pub fn remaining(&self, now_ms: u64) -> u32 {
        let elapsed = now_ms.saturating_sub(self.start_ms) / 1000;
        self.seconds.saturating_sub(elapsed as u32)
    }

    pub fn step(&mut self, now_ms: u64) -> CountdownStep {
        if self.done {
            return CountdownStep::None;
        }
        let n = self.remaining(now_ms);
        if n == 0 {
            self.done = true;
            return CountdownStep::Go;
        }
        if n < self.shown {
            self.shown = n;
            return CountdownStep::Tick(n);
        }
        CountdownStep::None
    }

    /// A countdown that should have ended a while ago (the caller checks the match is still paused).
    pub fn stalled(&self, now_ms: u64) -> bool {
        now_ms > self.start_ms + self.seconds as u64 * 1000 + STALL_GRACE_MS
    }
}

/// How long each countdown line stays in the message area: past the next second's line, which
/// replaces it, so it never flickers off.
pub const LINE_MS: u32 = 1_500;

/// The countdown in the message area, when the Pause Box is off ("Resuming in 3").
pub fn countdown_line(n: u32) -> String {
    if n == 0 {
        "Resuming".to_string()
    } else {
        format!("Resuming in {n}")
    }
}

/// Two players pressed Resume at about the same time and each heard of the other's countdown: the
/// lower player number keeps theirs, the other drops theirs, so one resume is sent. Every copy
/// decides the same. A player number that makes no sense keeps your own (at worst both resumes
/// are sent, and the second does nothing: the game ignores a resume when it is not paused).
pub fn theirs_wins(theirs: u32, local: u8) -> bool {
    theirs < local as u32
}

pub fn countdown_title(n: u32) -> String {
    if n == 0 {
        "RESUMING".to_string()
    } else {
        format!("RESUMING IN {n}")
    }
}

/// The plugin's messages to the other players (payloads after the host's envelope).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// A countdown of this many seconds started now.
    Start(u32),
    /// The countdown was cancelled.
    Cancel,
}

const MESSAGE_MAGIC: u8 = b'R';
const KIND_START: u8 = 1;
const KIND_CANCEL: u8 = 2;

pub fn encode(m: Message) -> Vec<u8> {
    match m {
        Message::Start(s) => vec![MESSAGE_MAGIC, KIND_START, s.clamp(1, 10) as u8],
        Message::Cancel => vec![MESSAGE_MAGIC, KIND_CANCEL],
    }
}

pub fn decode(b: &[u8]) -> Option<Message> {
    match b {
        [MESSAGE_MAGIC, KIND_START, s, ..] if (1..=10).contains(s) => {
            Some(Message::Start(*s as u32))
        }
        [MESSAGE_MAGIC, KIND_CANCEL, ..] => Some(Message::Cancel),
        _ => None,
    }
}

/// The tinted grey: the game's own grey shader (`y` from the same weights), mixed towards
/// `y * tint`. Same inputs as the stock one: `TexCoord` from the shared vertex shader and the
/// sampler `image`. Both uniforms are used, so neither is optimised out (the game aborts on an
/// unknown uniform name). NUL-terminated: the game passes it to glShaderSource without a length.
pub const TINT_FRAGMENT: &str = "#version 460 core
out vec4 FragColor;

in vec2 TexCoord;

uniform sampler2D image;
uniform vec3 tint;

void main()
{
\tvec4 color = texture(image, TexCoord);

\tfloat y = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
\tFragColor = vec4(mix(vec3(y), y * tint, 0.65), 1);
}
\0";
/// The uniform names, NUL-terminated for the game's setters.
pub const IMAGE_UNIFORM: &str = "image\0";
pub const TINT_UNIFORM: &str = "tint\0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_per_choice() {
        assert_eq!(
            effect_mode(GREY, PausedScreen::Default, true),
            GREY,
            "the stock grey-out"
        );
        assert_eq!(effect_mode(GREY, PausedScreen::Recolor, true), OFF);
        assert_eq!(effect_mode(GREY, PausedScreen::Tint, true), TINT_MODE);
        assert_eq!(
            effect_mode(GREY, PausedScreen::Tint, false),
            GREY,
            "no program: stock grey"
        );
    }

    #[test]
    fn other_modes_pass_through() {
        for screen in [
            PausedScreen::Default,
            PausedScreen::Recolor,
            PausedScreen::Tint,
        ] {
            for ready in [true, false] {
                assert_eq!(
                    effect_mode(OFF, screen, ready),
                    OFF,
                    "resume still switches it off"
                );
                assert_eq!(effect_mode(2, screen, ready), 2);
                assert_eq!(effect_mode(-1, screen, ready), -1);
            }
        }
    }

    #[test]
    fn stored_index_round_trips_and_defaults_to_default() {
        for screen in [
            PausedScreen::Default,
            PausedScreen::Recolor,
            PausedScreen::Tint,
        ] {
            assert_eq!(PausedScreen::from_index(screen.index()), screen);
            assert!(
                CHOICES.len() as i64 > screen.index(),
                "a label for every choice"
            );
        }
        assert_eq!(PausedScreen::DEFAULT, PausedScreen::Default);
        assert_eq!(PausedScreen::from_index(7), PausedScreen::Default);
        assert_eq!(PausedScreen::from_index(-1), PausedScreen::Default);
    }

    #[test]
    fn pauser_is_the_issuer_online_and_you_offline() {
        assert_eq!(pauser(true, 3, 0), Some(3));
        assert_eq!(
            pauser(false, 3, 0),
            Some(0),
            "offline the issuer byte is not trusted"
        );
        assert_eq!(pauser(true, 8, 0), None);
        assert_eq!(pauser(false, 0, 200), None);
    }

    #[test]
    fn tint_keeps_the_hue_at_full_brightness() {
        assert_eq!(tint_colour([0.64, 0.0, 0.0]), [1.0, 0.0, 0.0], "red");
        let black = tint_colour([0.16, 0.16, 0.24]);
        assert!(
            (black[2] - 1.0).abs() < 1e-6 && (black[0] - 2.0 / 3.0).abs() < 1e-3,
            "{black:?}"
        );
        assert_eq!(
            tint_colour([0.0, 0.0, 0.0]),
            [1.0, 1.0, 1.0],
            "no colour: no tint"
        );
        assert_eq!(
            tint_colour([f32::NAN, 2.0, -1.0]),
            [0.0, 1.0, 0.0],
            "bad values clamped"
        );
    }

    #[test]
    fn pauses_line_counts_used_and_left() {
        assert_eq!(pauses_line(3), "Pauses: 0 used, 3 left");
        assert_eq!(pauses_line(2), "Pauses: 1 used, 2 left");
        assert_eq!(pauses_line(0), "Pauses: 3 used, 0 left");
        assert_eq!(pauses_line(200), "Pauses: 0 used, 3 left", "garbage clamps");
    }

    #[test]
    fn names_offline_are_you_and_online_come_from_the_table() {
        assert_eq!(
            display_name(b"willeat4food\0junk", 2, false),
            "You",
            "offline: stale table"
        );
        assert_eq!(display_name(b"Maxx444\0junk", 4, true), "Maxx444");
        assert_eq!(display_name(b"\0abc", 0, true), "Player 1");
        assert_eq!(display_name(b"NoNul", 1, true), "NoNul");
    }

    #[test]
    fn pack_is_rgba_with_red_low() {
        assert_eq!(pack([1.0, 0.0, 0.0]), 0xff00_00ff);
        assert_eq!(pack([0.0, 0.0, 1.0]), 0xffff_0000);
        assert_eq!(pack([f32::NAN, 2.0, -1.0]), 0xff00_ff00);
    }

    #[test]
    fn box_fits_its_text_with_rows_in_order() {
        let l = box_layout(100.0, 50.0, 20.0, 120.0, 200.0, Some(300.0));
        let [x, y, w, h] = l.rect;
        assert_eq!((x, y), (100.0, 50.0));
        assert_eq!((w, h), box_size(20.0, 120.0, 200.0, Some(300.0)));
        assert!(
            w >= 300.0 + 2.0 * 40.0,
            "the widest line plus the frame: {w}"
        );
        let inside =
            |r: [f32; 4]| r[0] >= x && r[1] >= y && r[0] + r[2] <= x + w && r[1] + r[3] <= y + h;
        assert!(
            inside(l.title) && inside(l.name) && inside(l.swatch) && inside(l.count.unwrap()),
            "{l:?}"
        );
        assert!(
            l.title[1] < l.name[1] && l.name[1] < l.count.unwrap()[1],
            "rows top to bottom"
        );
        assert!(
            l.swatch[0] + l.swatch[2] < l.name[0],
            "swatch before the name"
        );
        let offline = box_layout(0.0, 0.0, 20.0, 120.0, 40.0, None);
        assert!(
            offline.count.is_none() && offline.rect[3] < h,
            "no count row offline"
        );
        let bad = box_size(f32::NAN, 0.0, 0.0, None);
        assert!(
            bad.0 > 0.0 && bad.1 > 0.0,
            "a bad font height still gives a box"
        );
    }

    #[test]
    fn hud_rects_scale_the_hud_layout_to_pixels() {
        // The tested setup: a 3840-pixel-wide window, a 960 x 540 HUD (scale 4).
        let r = hud_rects(3840.0, Some(0.5625), 960.0, 540.0, &HD_LAYOUT).unwrap();
        assert_eq!(r.screen, [0.0, 0.0, 3840.0, 2160.0]);
        assert_eq!(r.view, [704.0, 64.0, 3072.0, 2032.0]);
        assert_eq!(
            r.selection,
            [0.0, 640.0, 752.0, 704.0],
            "below the 160-unit minimap block"
        );
        assert_eq!(
            hud_rects(3840.0, None, 960.0, 540.0, &HD_LAYOUT),
            Some(r),
            "an unknown window shape is the HUD's own"
        );
        assert_eq!(hud_rects(f32::NAN, None, 960.0, 540.0, &HD_LAYOUT), None);
        assert_eq!(hud_rects(3840.0, None, 0.0, 540.0, &HD_LAYOUT), None);
    }

    #[test]
    fn the_classic_view_is_fitted_to_the_height_and_centred() {
        // The tested setup: a 3840 x 2400 window, the classic 800 x 600 HUD (scale 4, 320-pixel
        // black bars at the sides).
        let r = hud_rects(3840.0, Some(2400.0 / 3840.0), 800.0, 600.0, layout_for(0)).unwrap();
        assert_eq!(
            r.screen,
            [320.0, 0.0, 3200.0, 2400.0],
            "the picture between the black bars"
        );
        assert_eq!(r.view, [1024.0, 64.0, 2432.0, 2272.0]);
        assert_eq!(r.selection, [320.0, 640.0, 704.0, 688.0]);
        let (w, h, m) = (1120.0, 620.0, 40.0);
        for p in 0..BOX_POSITIONS.len() {
            let (x, y) = place_box(p, &r, w, h, m);
            assert!(
                x >= 320.0 && x + w <= 3520.0 && y >= 0.0 && y + h <= 2400.0,
                "{} stays on the classic screen",
                BOX_POSITIONS[p]
            );
        }
        assert_eq!(layout_for(1), &HD_LAYOUT);
        assert_eq!(layout_for(3), &HD_LAYOUT, "mid-switch");
    }

    #[test]
    fn every_position_lands_where_its_name_says() {
        let r = hud_rects(3840.0, None, 960.0, 540.0, &HD_LAYOUT).unwrap();
        let (w, h, m) = (1000.0, 600.0, 40.0);
        let [vx, vy, vw, vh] = r.view;
        let at = |p| place_box(p, &r, w, h, m);
        assert_eq!(at(0), (vx + m, vy + m), "top left");
        assert_eq!(at(1), ((vx + (vw - w) / 2.0).round(), vy + m), "top center");
        assert_eq!(at(2), (vx + vw - w - m, vy + m), "top right");
        assert_eq!(
            at(3),
            (vx + m, (vy + (vh - h) / 2.0 - 112.0).round()),
            "middle left"
        );
        assert_eq!(
            at(4),
            (
                (vx + (vw - w) / 2.0).round(),
                (vy + (vh - h) / 2.0 - 112.0).round()
            ),
            "center"
        );
        assert_eq!(
            at(5),
            (vx + vw - w - m, (vy + (vh - h) / 2.0 - 112.0).round()),
            "middle right"
        );
        assert_eq!(at(6), (vx + m, vy + vh - h - m), "bottom left");
        assert_eq!(
            at(7),
            ((vx + (vw - w) / 2.0).round(), vy + vh - h - m),
            "bottom center"
        );
        assert_eq!(at(8), (vx + vw - w - m, vy + vh - h - m), "bottom right");
        let (cx, cy) = at(9);
        assert_eq!(
            cx, 0.0,
            "over the selection display: wider than it, so kept inside the window"
        );
        assert_eq!(
            cy,
            640.0 + m - (m * 2.0 * 1.9f32).round(),
            "the frame a margin below the minimap"
        );
        let tested = place_box(9, &r, 1120.0, 620.0, 40.0).1;
        assert_eq!(
            tested, 528.0,
            "the tested window: art at about 682, 20 px below the border at 660"
        );
        for p in 0..BOX_POSITIONS.len() {
            let (x, y) = at(p);
            assert!(
                x >= 0.0 && y >= 0.0 && x + w <= 3840.0 && y + h <= 2160.0,
                "{p}: inside the window"
            );
        }
        assert_eq!(
            BOX_POSITIONS[BOX_POSITION_DEFAULT as usize],
            "Center (Default)"
        );
        assert_eq!(box_lock_condition(), "paused_screen!=1:1");
        assert_eq!(BOX_POSITIONS[9], "Over Selection Display");
        assert_eq!(box_position(10), BOX_POSITION_DEFAULT as usize);
        assert_eq!(box_position(-1), BOX_POSITION_DEFAULT as usize);
        assert_eq!(box_position(9), 9);
    }

    #[test]
    fn recolor_always_shows_the_box() {
        assert!(box_shown(false, PausedScreen::Recolor));
        assert!(!box_shown(false, PausedScreen::Default) && !box_shown(false, PausedScreen::Tint));
        assert!(box_shown(true, PausedScreen::Default) && box_shown(true, PausedScreen::Tint));
        assert_eq!(CHOICES[0], "Default");
    }

    #[test]
    fn countdown_ticks_five_to_one_then_goes_once() {
        let mut c = Countdown::new(10_000, COUNTDOWN_SECONDS, true);
        let mut steps = Vec::new();
        for t in (10_000..16_500).step_by(100) {
            match c.step(t) {
                CountdownStep::None => {}
                s => steps.push((t, s)),
            }
        }
        use CountdownStep::*;
        assert_eq!(
            steps,
            [
                (10_000, Tick(5)),
                (11_000, Tick(4)),
                (12_000, Tick(3)),
                (13_000, Tick(2)),
                (14_000, Tick(1)),
                (15_000, Go)
            ]
        );
        assert_eq!(c.remaining(12_500), 3);
        assert_eq!(countdown_title(3), "RESUMING IN 3");
        assert_eq!(countdown_title(0), "RESUMING");
    }

    #[test]
    fn a_late_frame_skips_to_the_current_number() {
        let mut c = Countdown::new(0, 5, false);
        assert_eq!(c.step(0), CountdownStep::Tick(5));
        assert_eq!(
            c.step(3_200),
            CountdownStep::Tick(2),
            "no backlog of ticks after a stall"
        );
        assert_eq!(c.step(9_000), CountdownStep::Go);
        assert_eq!(c.step(9_100), CountdownStep::None);
    }

    #[test]
    fn a_countdown_stalls_after_the_grace() {
        let c = Countdown::new(0, 5, false);
        assert!(!c.stalled(5_000 + STALL_GRACE_MS));
        assert!(c.stalled(5_001 + STALL_GRACE_MS));
    }

    #[test]
    fn the_lower_player_keeps_their_countdown() {
        assert!(theirs_wins(0, 1));
        assert!(!theirs_wins(2, 1));
        assert!(!theirs_wins(1, 1), "never both dropped");
        assert!(!theirs_wins(0xffff_ffff, 3), "nonsense keeps your own");
        assert_eq!(countdown_line(4), "Resuming in 4");
    }

    #[test]
    fn messages_round_trip_and_junk_is_refused() {
        for m in [Message::Start(5), Message::Cancel] {
            assert_eq!(decode(&encode(m)), Some(m));
        }
        assert_eq!(decode(&[b'R', 1, 0]), None, "zero seconds");
        assert_eq!(decode(&[b'R', 1, 99]), None);
        assert_eq!(decode(&[b'X', 1, 5]), None);
        assert_eq!(decode(&[]), None);
    }

    #[test]
    fn shader_text_is_terminated_and_uses_both_uniforms() {
        for s in [TINT_FRAGMENT, IMAGE_UNIFORM, TINT_UNIFORM] {
            assert!(s.ends_with('\0') && s.matches('\0').count() == 1, "{s:?}");
        }
        assert!(TINT_FRAGMENT.starts_with("#version 460 core\n"));
        assert!(TINT_FRAGMENT.contains("texture(image, TexCoord)"));
        assert!(TINT_FRAGMENT.contains("y * tint"));
        assert!(
            TINT_FRAGMENT.contains("uniform sampler2D image;")
                && TINT_FRAGMENT.contains("uniform vec3 tint;")
        );
    }
}
