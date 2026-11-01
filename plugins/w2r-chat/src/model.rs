//! The chat plugin's pure state: settings, the message backlog, fade and colour rules. No game
//! access, so it is unit-tested on any target. Constants are wc2r-plus `chat.cpp`'s.

use std::collections::VecDeque;
use std::ffi::CString;

pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

pub const HIGHLIGHT: u32 = rgba(255, 204, 0, 255);
pub const TEXT_GREY: u32 = rgba(0x92, 0x92, 0x92, 255);
pub const CURSOR_COLOUR: u32 = rgba(166, 141, 105, 235);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
    #[default]
    System,
    All,
    Team,
    Allies,
    Enemies,
    Private,
}

impl Channel {
    /// The tag shown before a line of this channel (empty for system lines).
    pub fn label(self) -> &'static str {
        match self {
            Channel::System => "",
            Channel::All => "[All]",
            Channel::Team => "[Team]",
            Channel::Allies => "[Allies]",
            Channel::Enemies => "[Enemies]",
            Channel::Private => "[Private]",
        }
    }
}

/// A channel whose colour the player can change (wc2r-plus 1.1.0 `kChannelColourSettings`).
pub struct ColourSpec {
    pub channel: Channel,
    /// The settings key; the value is six hex digits, `rrggbb`.
    pub key: &'static str,
    pub label: &'static str,
    pub default_hex: &'static str,
}

pub const CHANNEL_COLOURS: [ColourSpec; 3] = [
    ColourSpec {
        channel: Channel::Team,
        key: "team_hex",
        label: "[Team] color (hex)",
        default_hex: "25e65f",
    },
    ColourSpec {
        channel: Channel::All,
        key: "all_hex",
        label: "[All] color (hex)",
        default_hex: "929292",
    },
    ColourSpec {
        channel: Channel::Allies,
        key: "allies_hex",
        label: "[Allies] color (hex)",
        default_hex: "69a7ff",
    },
];

/// Exactly six hex digits, no `#` and no spaces, as wc2r-plus `hex_colour::Parse`: `rrggbb`.
pub fn parse_hex(text: &str) -> Option<u32> {
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

/// A hex setting as a drawable colour; anything invalid gives the channel's default.
pub fn hex_to_colour(text: &str, default_hex: &str) -> u32 {
    let rgb = parse_hex(text)
        .or_else(|| parse_hex(default_hex))
        .unwrap_or(0xffffff);
    rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}

/// The game's target mode for a team message to a custom recipient mask.
pub const TEAM_TARGET_MODE: u8 = 8;

/// The channel a received message belongs to, from the target mode it was sent with.
pub fn channel_of(target_mode: u8) -> Channel {
    match target_mode {
        TEAM_TARGET_MODE => Channel::Team,
        3 => Channel::Allies,
        4 => Channel::Enemies,
        5 => Channel::Private,
        _ => Channel::All,
    }
}

/// The target mode the game's compose box is put in for a channel (it only knows modes 2, 3, 6).
pub fn compose_mode_of(channel: Channel) -> u8 {
    match channel {
        Channel::All => 2,
        Channel::Allies => 3,
        _ => 6,
    }
}

/// Modifier bits of a key event (wc2r-plus `struct_offsets.h`).
pub const MOD_SHIFT: u8 = 0x03;
pub const MOD_CTRL: u8 = 0x04;

/// The channel Enter opens: Shift is `[All]`, Ctrl is `[Allies]`, plain is the channel composed in
/// last (`last`, `System` for none yet) or `[Team]`; a channel with nobody in it falls back to
/// `[All]`.
pub fn open_channel(modifiers: u8, has_team: bool, has_allies: bool, last: Channel) -> Channel {
    if modifiers & MOD_SHIFT != 0 {
        Channel::All
    } else if modifiers & MOD_CTRL != 0 {
        if has_allies {
            Channel::Allies
        } else {
            Channel::All
        }
    } else if last == Channel::All {
        Channel::All
    } else if last == Channel::Allies && has_allies {
        Channel::Allies
    } else if has_team {
        Channel::Team
    } else {
        Channel::All
    }
}

/// Tab: the next of Team, All, Allies after `current`, skipping empty channels; `current` itself
/// when nothing else is available.
pub fn next_channel(current: Channel, has_team: bool, has_allies: bool) -> Channel {
    const ORDER: [Channel; 3] = [Channel::Team, Channel::All, Channel::Allies];
    let at = ORDER.iter().position(|&c| c == current).unwrap_or(0);
    for step in 1..=3 {
        let next = ORDER[(at + step) % 3];
        if (next == Channel::Team && !has_team) || (next == Channel::Allies && !has_allies) {
            continue;
        }
        return next;
    }
    current
}

/// The bit mask of other in-match players that are allied to `me` (`allied[i]`) and, for the team
/// mask, on `me`'s lobby team too. `in_match[i]` is owner 0 or 1.
pub fn recipient_mask(
    me: usize,
    in_match: &[bool; PLAYER_COUNT],
    allied: &[bool; PLAYER_COUNT],
    same_team: Option<&[bool; PLAYER_COUNT]>,
) -> u8 {
    if me >= PLAYER_COUNT {
        return 0;
    }
    let mut mask = 0u8;
    for i in 0..PLAYER_COUNT {
        if i != me && in_match[i] && allied[i] && same_team.is_none_or(|t| t[i]) {
            mask |= 1 << i;
        }
    }
    mask
}

/// A numeric setting: key, default, inclusive range (wc2r-plus `RegisterChatSettings`).
pub struct Spec {
    pub key: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
}

pub const ENABLED: Spec = Spec {
    key: "enabled",
    default: 1,
    min: 0,
    max: 1,
};
pub const LINES: Spec = Spec {
    key: "lines",
    default: 8,
    min: 2,
    max: 20,
};
pub const FADE: Spec = Spec {
    key: "fade",
    default: 7,
    min: 0,
    max: 60,
};
pub const BACKLOG: Spec = Spec {
    key: "backlog",
    default: 100,
    min: 25,
    max: 500,
};
pub const BG: Spec = Spec {
    key: "bg",
    default: 100,
    min: 0,
    max: 255,
};
pub const TIMESTAMPS: Spec = Spec {
    key: "timestamps",
    default: 0,
    min: 0,
    max: 1,
};
pub const NAME_COLOURS: Spec = Spec {
    key: "name_colours",
    default: 1,
    min: 0,
    max: 1,
};
pub const CHANNELS: Spec = Spec {
    key: "channels",
    default: 1,
    min: 0,
    max: 1,
};
pub const CLICK_UNFOCUS: Spec = Spec {
    key: "click_unfocus",
    default: 0,
    min: 0,
    max: 1,
};
pub const LOG: Spec = Spec {
    key: "log",
    default: 0,
    min: 0,
    max: 1,
};

impl Spec {
    pub fn clamp(&self, v: i64) -> i64 {
        v.clamp(self.min, self.max)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Channel colours in `CHANNEL_COLOURS` order, as drawable colours.
    pub channel_colours: [u32; 3],
    pub enabled: bool,
    pub lines: i32,
    pub fade_seconds: i32,
    pub backlog: usize,
    pub bg_alpha: i32,
    pub timestamps: bool,
    pub log: bool,
    pub name_colours: bool,
    pub channels: bool,
    pub click_unfocus: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            channel_colours: CHANNEL_COLOURS.map(|c| hex_to_colour(c.default_hex, c.default_hex)),
            enabled: ENABLED.default != 0,
            lines: LINES.default as i32,
            fade_seconds: FADE.default as i32,
            backlog: BACKLOG.default as usize,
            bg_alpha: BG.default as i32,
            timestamps: TIMESTAMPS.default != 0,
            log: LOG.default != 0,
            name_colours: NAME_COLOURS.default != 0,
            channels: CHANNELS.default != 0,
            click_unfocus: CLICK_UNFOCUS.default != 0,
        }
    }
}

impl Settings {
    /// Applies one stored value, clamped to its range; an unknown key is ignored.
    pub fn apply(&mut self, key: &str, value: i64) {
        match key {
            "enabled" => self.enabled = ENABLED.clamp(value) != 0,
            "lines" => self.lines = LINES.clamp(value) as i32,
            "fade" => self.fade_seconds = FADE.clamp(value) as i32,
            "backlog" => self.backlog = BACKLOG.clamp(value) as usize,
            "bg" => self.bg_alpha = BG.clamp(value) as i32,
            "timestamps" => self.timestamps = TIMESTAMPS.clamp(value) != 0,
            "log" => self.log = LOG.clamp(value) != 0,
            "name_colours" => self.name_colours = NAME_COLOURS.clamp(value) != 0,
            "channels" => self.channels = CHANNELS.clamp(value) != 0,
            "click_unfocus" => self.click_unfocus = CLICK_UNFOCUS.clamp(value) != 0,
            _ => {}
        }
    }

    /// Applies one stored hex colour. False when the text is not six hex digits (the channel
    /// then uses its default, so a half-typed value never shows a wrong colour); an unknown key
    /// is ignored and reported as fine.
    pub fn apply_hex(&mut self, key: &str, text: &str) -> bool {
        let Some(i) = CHANNEL_COLOURS.iter().position(|c| c.key == key) else {
            return true;
        };
        self.channel_colours[i] = hex_to_colour(text, CHANNEL_COLOURS[i].default_hex);
        parse_hex(text).is_some()
    }
}

/// The colour of every kind of line for one frame (wc2r-plus `LinePalette`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    by_channel: [u32; 6],
}

impl Palette {
    /// With enhanced channels on, the three channels the player can recolour use their settings
    /// and the rest are grey. With them off every chat line takes the game's own text colour
    /// (`game_text`), so the mod does not repaint chat it is not managing. System lines keep the
    /// highlight colour either way.
    pub fn new(settings: &Settings, game_text: u32) -> Palette {
        let mut by_channel = [TEXT_GREY; 6];
        by_channel[Channel::System as usize] = HIGHLIGHT;
        if settings.enabled && settings.channels {
            for (spec, &c) in CHANNEL_COLOURS.iter().zip(&settings.channel_colours) {
                by_channel[spec.channel as usize] = c;
            }
        } else {
            for (i, c) in by_channel.iter_mut().enumerate() {
                if i != Channel::System as usize {
                    *c = game_text;
                }
            }
        }
        Palette { by_channel }
    }

    pub fn of(&self, channel: Channel) -> u32 {
        self.by_channel[channel as usize]
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    /// NUL-terminated for the game's C-string widgets.
    pub text: CString,
    pub highlight: bool,
    pub arrived_ms: u64,
    pub channel: Channel,
    /// Parts of the text drawn in a player's colour, in order, not overlapping.
    pub names: Vec<NameSpan>,
}

impl Message {
    pub fn new(text: &[u8], highlight: bool, channel: Channel, now_ms: u64) -> Option<Message> {
        // The game's text, byte for byte; a NUL would end the line early in its widgets, so cut
        // there.
        let mut bytes: Vec<u8> = text.iter().copied().take_while(|&b| b != 0).collect();
        // Some game messages end in a line break, which would draw as an empty line below.
        while matches!(bytes.last(), Some(b'\r' | b'\n')) {
            bytes.pop();
        }
        if bytes.is_empty() {
            return None;
        }
        Some(Message {
            text: CString::new(bytes).expect("no NUL"),
            highlight,
            arrived_ms: now_ms,
            channel,
            names: Vec::new(),
        })
    }

    /// The same message with colour spans (byte offsets into the text as stored).
    pub fn with_names(mut self, names: Vec<NameSpan>) -> Message {
        let len = self.text.as_bytes().len();
        self.names = names
            .into_iter()
            .filter(|n| n.len > 0 && n.start + n.len <= len)
            .collect();
        self
    }

    pub fn colour(&self, palette: &Palette) -> u32 {
        if self.highlight {
            HIGHLIGHT
        } else {
            palette.of(self.channel)
        }
    }
}

/// The newest `cap` messages, oldest dropped first.
#[derive(Default)]
pub struct Backlog {
    items: VecDeque<Message>,
    scroll_to_bottom: bool,
}

impl Backlog {
    pub fn push(&mut self, m: Message, cap: usize) {
        self.items.push_back(m);
        while self.items.len() > cap {
            self.items.pop_front();
        }
        self.scroll_to_bottom = true;
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.scroll_to_bottom = false;
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn snapshot(&self) -> Vec<Message> {
        self.items.iter().cloned().collect()
    }

    /// Newest first, for the recent-lines view.
    pub fn recent_first(&self) -> impl Iterator<Item = &Message> {
        self.items.iter().rev()
    }

    pub fn request_scroll(&mut self) {
        self.scroll_to_bottom = true;
    }

    /// True once after a push or reveal.
    pub fn take_scroll(&mut self) -> bool {
        std::mem::replace(&mut self.scroll_to_bottom, false)
    }
}

pub const FADE_OUT_MS: u64 = 1000;

/// 1.0 while the message is young, then a linear fade over a second to 0.
pub fn visibility(arrived_ms: u64, now_ms: u64, hold_ms: u64) -> f32 {
    let age = now_ms.saturating_sub(arrived_ms);
    if age < hold_ms {
        1.0
    } else if age >= hold_ms + FADE_OUT_MS {
        0.0
    } else {
        1.0 - (age - hold_ms) as f32 / FADE_OUT_MS as f32
    }
}

/// Scales the alpha byte of an `rgba` colour.
pub fn faded(colour: u32, alpha: f32) -> u32 {
    if alpha >= 1.0 {
        return colour;
    }
    let a = ((colour >> 24) as f32 * alpha) as u32;
    (colour & 0x00ff_ffff) | (a << 24)
}

/// The game multiplies text colour channels by a style factor; alpha is untouched.
pub fn apply_colour_factor(colour: u32, factor: f32) -> u32 {
    if factor == 1.0 {
        return colour;
    }
    let ch = |shift: u32| (((colour >> shift) & 0xff) as f32 * factor) as i32 as u8 as u32;
    (colour & 0xff00_0000) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Whether the full history (not just recent lines) is shown: fade is off, the player is
/// typing, or the game is paused.
pub fn revealed(fade_seconds: i32, chat_target_mode: u8, pause_flags: u8) -> bool {
    fade_seconds <= 0 || chat_target_mode != 0 || pause_flags & 1 != 0
}

/// Local wall-clock time, as the game machine shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// `[HH:MM] `, the prefix "Show Timestamps" puts on a message.
pub fn timestamp_prefix(t: &LocalTime) -> String {
    format!("[{:02}:{:02}] ", t.hour, t.minute)
}

/// One line of the disk log: `[YYYY-MM-DD HH:MM:SS] text`.
pub fn log_line(t: &LocalTime, text: &str) -> String {
    format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] {text}\n",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// The per-launch log file name; fits the host's data-file rule.
pub fn log_file_name(t: &LocalTime) -> String {
    format!(
        "chat-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.log",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

pub const LOG_MATCH_MARKER: &str = "\n=== new match ===\n";

/// The game's player name slot: NUL-terminated within the slot. A name that fills the slot (no
/// terminator) or is empty is not trusted.
pub fn player_name(slot: &[u8]) -> Option<String> {
    let n = slot.iter().position(|&b| b == 0)?;
    (n > 0).then(|| String::from_utf8_lossy(&slot[..n]).into_owned())
}

/// "X paused the game." (with the remaining pauses in a networked match) or "X resumed the game."
/// Without a name (offline, where the game leaves the last online match's names in PlayerNames)
/// it is "Game paused." or "Game resumed."
pub fn pause_line(name: Option<&str>, paused: bool, remaining: Option<u8>) -> String {
    let Some(name) = name else {
        return if paused {
            "Game paused."
        } else {
            "Game resumed."
        }
        .to_string();
    };
    if !paused {
        return format!("{name} resumed the game.");
    }
    match remaining {
        Some(n) => format!(
            "{name} paused the game. ({n} pause{} remaining)",
            if n == 1 { "" } else { "s" }
        ),
        None => format!("{name} paused the game."),
    }
}

// ---- player names and colours --------------------------------------------------------------

/// The game has eight players and eight colours.
pub const PLAYER_COUNT: usize = 8;
pub const BLACK: u8 = 5;
/// The black player's name is drawn grey so it stays readable on the dark panel.
pub const BLACK_NAME: u32 = rgba(0x20, 0x20, 0x20, 255);

/// Stock player colours (wc2r-plus `team_colours::kColours`), index = the game's colour index.
pub const PLAYER_COLOURS: [u32; PLAYER_COUNT] = [
    rgba(196, 40, 40, 255),
    rgba(48, 72, 200, 255),
    rgba(0, 152, 152, 255),
    rgba(128, 48, 168, 255),
    rgba(232, 132, 24, 255),
    rgba(32, 32, 32, 255),
    rgba(236, 236, 236, 255),
    rgba(232, 212, 48, 255),
];

/// The black player's name colour from the game's table entry (0..1 floats): the stock value keeps
/// the readable grey, anything else (a recolour) is drawn as is.
pub fn black_name(rgb: [f32; 3]) -> u32 {
    let b = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    if b == [32, 32, 32] {
        BLACK_NAME
    } else {
        rgba(b[0], b[1], b[2], 255)
    }
}

/// The colour a name is drawn in; an out-of-range index falls back to `fallback`.
pub fn name_colour(index: u8, fallback: u32) -> u32 {
    match index {
        BLACK => BLACK_NAME,
        i if (i as usize) < PLAYER_COUNT => PLAYER_COLOURS[i as usize],
        _ => fallback,
    }
}

/// `len` bytes at `start` of a message's text are a player's name in colour `colour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameSpan {
    pub start: usize,
    pub len: usize,
    pub colour: u8,
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Where the sender's name sits in a chat line: a whole name followed by `": "`.
pub fn find_sender_name(line: &[u8], name: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = find_from(line, name, from) {
        let end = at + name.len();
        let preceded = at > 0 && is_name_byte(line[at - 1]);
        if !preceded && line[end..].starts_with(b": ") {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Every unique human player name in `line`, longest first, whole words only, no overlaps
/// (`FindPlayerNames`). `humans` is (name, colour index) for each human of two or more characters
/// with a real colour.
pub fn find_player_names(line: &[u8], humans: &[(Vec<u8>, u8)]) -> Vec<NameSpan> {
    let mut candidates: Vec<&(Vec<u8>, u8)> = humans
        .iter()
        .filter(|c| humans.iter().filter(|o| o.0 == c.0).count() == 1)
        .collect();
    candidates.sort_by_key(|c| core::cmp::Reverse(c.0.len()));
    let mut spans: Vec<NameSpan> = Vec::new();
    for (name, colour) in candidates {
        let mut from = 0;
        while let Some(at) = find_from(line, name, from) {
            let end = at + name.len();
            from = at + 1;
            if at > 0 && is_name_byte(line[at - 1]) {
                continue;
            }
            if end < line.len() && is_name_byte(line[end]) {
                continue;
            }
            if spans.iter().any(|o| at < o.start + o.len && o.start < end) {
                continue;
            }
            spans.push(NameSpan {
                start: at,
                len: name.len(),
                colour: *colour,
            });
        }
    }
    spans.sort_by_key(|s| s.start);
    spans
}

/// A whisper arrives as `<name> text`; it is shown as `[name] text`.
pub fn square_whisper_brackets(line: &mut [u8], names: &[NameSpan]) {
    if line.first() != Some(&b'<') {
        return;
    }
    let first_close = line.iter().position(|&b| b == b'>');
    for n in names {
        let close = n.start + n.len;
        if close < line.len() && line[close] == b'>' && first_close == Some(close) {
            line[0] = b'[';
            line[close] = b']';
            return;
        }
    }
}

/// The game's name slot as bytes: up to the NUL, not empty, and it must have a terminator.
pub fn name_bytes(slot: &[u8]) -> Option<Vec<u8>> {
    let n = slot.iter().position(|&b| b == 0)?;
    (n > 0).then(|| slot[..n].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(text: &str, at: u64) -> Message {
        Message::new(text.as_bytes(), false, Channel::All, at).unwrap()
    }

    #[test]
    fn colours_pack_like_wc2r_plus() {
        assert_eq!(rgba(255, 204, 0, 255), 0xff00_ccff);
        assert_eq!(HIGHLIGHT, 0xff00_ccff);
        let p = Palette::new(&Settings::default(), 0);
        assert_eq!(p.of(Channel::Team), rgba(0x25, 0xe6, 0x5f, 255));
        assert_eq!(p.of(Channel::Allies), rgba(0x69, 0xa7, 0xff, 255));
        assert_eq!(p.of(Channel::All), TEXT_GREY);
        assert_eq!(p.of(Channel::System), HIGHLIGHT);
    }

    #[test]
    fn hex_is_exactly_six_digits() {
        assert_eq!(parse_hex("25e65f"), Some(0x25e65f));
        assert_eq!(parse_hex("FFAA00"), Some(0xffaa00));
        for bad in [
            "", "25e65", "25e65ff", "#25e65f", "25e65g", " 25e65f", "+5e65f",
        ] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_bad_hex_setting_falls_back_to_the_default() {
        let mut s = Settings::default();
        assert!(s.apply_hex("team_hex", "ff8800"));
        assert_eq!(
            Palette::new(&s, 0).of(Channel::Team),
            rgba(0xff, 0x88, 0x00, 255)
        );
        assert!(!s.apply_hex("team_hex", "ff88"));
        assert_eq!(
            Palette::new(&s, 0).of(Channel::Team),
            rgba(0x25, 0xe6, 0x5f, 255)
        );
        assert!(
            s.apply_hex("not_a_colour", "zzz"),
            "unknown keys are ignored"
        );
    }

    #[test]
    fn channels_off_uses_the_game_text_colour_except_for_system_lines() {
        let game = rgba(250, 250, 250, 255);
        let mut s = Settings::default();
        s.channels = false;
        let p = Palette::new(&s, game);
        assert_eq!(p.of(Channel::Team), game);
        assert_eq!(p.of(Channel::All), game);
        assert_eq!(p.of(Channel::System), HIGHLIGHT);
        s.channels = true;
        s.enabled = false;
        assert_eq!(
            Palette::new(&s, game).of(Channel::Allies),
            game,
            "plugin off counts as channels off"
        );
    }

    #[test]
    fn settings_default_clamp_and_ignore_the_unknown() {
        let mut s = Settings::default();
        assert_eq!(
            (s.lines, s.fade_seconds, s.backlog, s.bg_alpha),
            (8, 7, 100, 100)
        );
        assert!(s.enabled);
        s.apply("lines", 99);
        s.apply("fade", -5);
        s.apply("backlog", 1);
        s.apply("bg", 300);
        s.apply("enabled", 0);
        s.apply("nonsense", 1);
        assert_eq!(
            (s.lines, s.fade_seconds, s.backlog, s.bg_alpha),
            (20, 0, 25, 255)
        );
        assert!(!s.enabled);
    }

    #[test]
    fn the_backlog_keeps_the_newest_messages_up_to_the_cap() {
        let mut b = Backlog::default();
        for i in 0..30 {
            b.push(msg(&format!("m{i}"), i), 25);
        }
        assert_eq!(b.len(), 25);
        let snap = b.snapshot();
        assert_eq!(snap.first().unwrap().text.to_str().unwrap(), "m5");
        assert_eq!(snap.last().unwrap().text.to_str().unwrap(), "m29");
        assert_eq!(
            b.recent_first().next().unwrap().text.to_str().unwrap(),
            "m29"
        );
        // Lowering the cap drops from the front on the next push.
        b.push(msg("x", 99), 3);
        assert_eq!(b.len(), 3);
        assert!(b.take_scroll());
        assert!(!b.take_scroll(), "only once");
        b.clear();
        assert!(b.is_empty() && !b.take_scroll());
    }

    #[test]
    fn empty_and_nul_text_make_no_message() {
        assert!(Message::new(b"", false, Channel::All, 0).is_none());
        assert!(Message::new(b"\0abc", false, Channel::All, 0).is_none());
        let m = Message::new(b"ab\0cd", false, Channel::All, 0).unwrap();
        assert_eq!(m.text.to_str().unwrap(), "ab");
    }

    #[test]
    fn trailing_line_breaks_are_dropped() {
        let m = Message::new(b"Building Destroyed\r\n", false, Channel::System, 0).unwrap();
        assert_eq!(m.text.to_str().unwrap(), "Building Destroyed");
        assert!(Message::new(b"\r\n", false, Channel::All, 0).is_none());
    }

    #[test]
    fn highlight_wins_over_the_channel_colour() {
        let p = Palette::new(&Settings::default(), 0);
        let mut m = msg("x", 0);
        assert_eq!(m.colour(&p), TEXT_GREY);
        m.highlight = true;
        assert_eq!(m.colour(&p), HIGHLIGHT);
    }

    #[test]
    fn visibility_holds_then_fades_over_a_second() {
        assert_eq!(visibility(1000, 1000, 7000), 1.0);
        assert_eq!(visibility(1000, 7999, 7000), 1.0);
        assert!((visibility(1000, 8500, 7000) - 0.5).abs() < 1e-6);
        assert_eq!(visibility(1000, 9000, 7000), 0.0);
        assert_eq!(
            visibility(5000, 100, 7000),
            1.0,
            "a clock that went backwards"
        );
    }

    #[test]
    fn alpha_fading_and_the_colour_factor() {
        assert_eq!(faded(0xff10_2030, 1.0), 0xff10_2030);
        assert_eq!(faded(0xff10_2030, 0.5), 0x7f10_2030);
        assert_eq!(faded(0xff10_2030, 0.0), 0x0010_2030);
        assert_eq!(apply_colour_factor(0xff10_2030, 1.0), 0xff10_2030);
        assert_eq!(apply_colour_factor(0xff10_2030, 0.5), 0xff08_1018);
    }

    #[test]
    fn the_history_is_revealed_when_fade_is_off_typing_or_paused() {
        assert!(!revealed(7, 0, 0));
        assert!(revealed(0, 0, 0));
        assert!(revealed(7, 2, 0));
        assert!(revealed(7, 0, 1));
        assert!(!revealed(7, 0, 0b10), "only bit 0 is the pause flag");
    }

    fn t() -> LocalTime {
        LocalTime {
            year: 2026,
            month: 10,
            day: 4,
            hour: 9,
            minute: 5,
            second: 7,
        }
    }

    #[test]
    fn timestamps_log_lines_and_file_names_follow_wc2r_plus() {
        assert_eq!(timestamp_prefix(&t()), "[09:05] ");
        assert_eq!(log_line(&t(), "hi"), "[2026-10-04 09:05:07] hi\n");
        assert_eq!(log_file_name(&t()), "chat-2026-10-04_09-05-07.log");
        assert_eq!(LOG_MATCH_MARKER, "\n=== new match ===\n");
    }

    #[test]
    fn pause_lines_pluralise_and_skip_the_count_offline() {
        assert_eq!(
            pause_line(Some("Ann"), true, Some(2)),
            "Ann paused the game. (2 pauses remaining)"
        );
        assert_eq!(
            pause_line(Some("Ann"), true, Some(1)),
            "Ann paused the game. (1 pause remaining)"
        );
        assert_eq!(
            pause_line(Some("Ann"), true, Some(0)),
            "Ann paused the game. (0 pauses remaining)"
        );
        assert_eq!(pause_line(Some("Ann"), true, None), "Ann paused the game.");
        assert_eq!(
            pause_line(Some("Ann"), false, Some(2)),
            "Ann resumed the game."
        );
    }

    #[test]
    fn offline_pause_lines_name_no_player() {
        assert_eq!(pause_line(None, true, None), "Game paused.");
        assert_eq!(pause_line(None, false, None), "Game resumed.");
        assert_eq!(pause_line(None, true, Some(2)), "Game paused.");
    }

    #[test]
    fn player_names_need_a_terminator_and_text() {
        assert_eq!(player_name(b"Ann\0junk"), Some("Ann".into()));
        assert_eq!(player_name(b"\0abc"), None);
        assert_eq!(
            player_name(b"nonul"),
            None,
            "fills the slot with no terminator"
        );
    }

    #[test]
    fn the_new_settings_default_off_and_clamp() {
        let mut s = Settings::default();
        assert!(!s.timestamps && !s.log);
        s.apply("timestamps", 1);
        s.apply("log", 7);
        assert!(s.timestamps && s.log);
        s.apply("log", -3);
        assert!(!s.log);
    }

    fn span(start: usize, len: usize, colour: u8) -> NameSpan {
        NameSpan { start, len, colour }
    }

    #[test]
    fn the_sender_is_found_as_a_whole_name_before_a_colon() {
        assert_eq!(find_sender_name(b"Ann: hello", b"Ann"), Some(0));
        assert_eq!(find_sender_name(b"[All] Ann: hi", b"Ann"), Some(6));
        assert_eq!(
            find_sender_name(b"Joann: hi", b"Ann"),
            None,
            "inside another name"
        );
        assert_eq!(find_sender_name(b"Ann says hi", b"Ann"), None, "no colon");
        // The first hit is a quote of the name, the second is the sender.
        assert_eq!(find_sender_name(b"hi Ann, Ann: yes", b"Ann"), Some(8));
        assert_eq!(find_sender_name(b"x", b"Ann"), None);
        assert_eq!(
            find_sender_name(b"Ann:", b"Ann"),
            None,
            "needs the space too"
        );
    }

    #[test]
    fn player_names_in_a_system_line_are_found_longest_first_without_overlaps() {
        let humans = vec![
            (b"Ann".to_vec(), 0),
            (b"Annabel".to_vec(), 1),
            (b"Bob".to_vec(), 2),
        ];
        let line = b"Annabel and Ann killed Bob. Bobby ran.";
        assert_eq!(
            find_player_names(line, &humans),
            vec![span(0, 7, 1), span(12, 3, 0), span(23, 3, 2)]
        );
    }

    #[test]
    fn names_two_players_share_are_not_coloured() {
        let humans = vec![
            (b"Sam".to_vec(), 0),
            (b"Sam".to_vec(), 1),
            (b"Zed".to_vec(), 2),
        ];
        assert_eq!(
            find_player_names(b"Sam met Zed", &humans),
            vec![span(8, 3, 2)]
        );
    }

    #[test]
    fn whispers_get_square_brackets_only_around_a_known_name() {
        let mut line = b"<Ann> psst".to_vec();
        square_whisper_brackets(&mut line, &[span(1, 3, 0)]);
        assert_eq!(line, b"[Ann] psst");
        let mut other = b"<Ann> psst".to_vec();
        square_whisper_brackets(&mut other, &[]);
        assert_eq!(other, b"<Ann> psst");
        let mut later = b"<x> a <Ann> b".to_vec();
        square_whisper_brackets(&mut later, &[span(7, 3, 0)]);
        assert_eq!(later, b"<x> a <Ann> b", "the first > is not the name's");
        let mut plain = b"hello".to_vec();
        square_whisper_brackets(&mut plain, &[span(0, 5, 0)]);
        assert_eq!(plain, b"hello");
    }

    #[test]
    fn name_colours_use_the_stock_table_and_grey_for_black() {
        assert_eq!(name_colour(0, 7), rgba(196, 40, 40, 255));
        assert_eq!(name_colour(7, 7), rgba(232, 212, 48, 255));
        assert_eq!(name_colour(BLACK, 7), BLACK_NAME);
        assert_eq!(name_colour(8, 7), 7, "no colour: the fallback");
        assert_eq!(PLAYER_COLOURS.len(), PLAYER_COUNT);
    }

    #[test]
    fn name_slots_need_text_and_a_terminator() {
        assert_eq!(name_bytes(b"Ann\0xyz"), Some(b"Ann".to_vec()));
        assert_eq!(name_bytes(b"\0abc"), None);
        assert_eq!(name_bytes(b"full"), None);
    }

    #[test]
    fn spans_outside_the_text_are_dropped() {
        let m = Message::new(b"Ann: hi", false, Channel::All, 0)
            .unwrap()
            .with_names(vec![span(0, 3, 0), span(5, 9, 1), span(2, 0, 2)]);
        assert_eq!(m.names, vec![span(0, 3, 0)]);
    }

    #[test]
    fn black_name_follows_a_recolour() {
        assert_eq!(black_name([32.0 / 255.0; 3]), BLACK_NAME);
        let pink = [0xe8 as f32 / 255.0, 0x7b as f32 / 255.0, 0xdd as f32 / 255.0];
        assert_eq!(black_name(pink), rgba(0xe8, 0x7b, 0xdd, 255));
    }

    #[test]
    fn name_colouring_defaults_on() {
        let mut s = Settings::default();
        assert!(s.name_colours);
        s.apply("name_colours", 0);
        assert!(!s.name_colours);
    }

    #[test]
    fn channels_map_modes_and_labels() {
        assert_eq!(channel_of(8), Channel::Team);
        assert_eq!(channel_of(3), Channel::Allies);
        assert_eq!(channel_of(4), Channel::Enemies);
        assert_eq!(channel_of(5), Channel::Private);
        assert_eq!(channel_of(2), Channel::All);
        assert_eq!(compose_mode_of(Channel::All), 2);
        assert_eq!(compose_mode_of(Channel::Allies), 3);
        assert_eq!(compose_mode_of(Channel::Team), 6);
        assert_eq!(Channel::Team.label(), "[Team]");
        assert_eq!(Channel::System.label(), "");
    }

    #[test]
    fn enter_opens_by_modifier_and_falls_back_to_all() {
        let s = Channel::System;
        assert_eq!(open_channel(0, true, true, s), Channel::Team);
        assert_eq!(open_channel(0, false, true, s), Channel::All);
        assert_eq!(open_channel(MOD_CTRL, true, true, s), Channel::Allies);
        assert_eq!(open_channel(MOD_CTRL, true, false, s), Channel::All);
        assert_eq!(open_channel(MOD_SHIFT, true, true, s), Channel::All);
        assert_eq!(open_channel(MOD_SHIFT | MOD_CTRL, true, true, s), Channel::All);
        // Plain Enter reopens the channel used last, when it still has people in it.
        assert_eq!(open_channel(0, true, true, Channel::Allies), Channel::Allies);
        assert_eq!(open_channel(0, true, false, Channel::Allies), Channel::Team);
        assert_eq!(open_channel(0, true, true, Channel::All), Channel::All);
        assert_eq!(open_channel(0, true, true, Channel::Team), Channel::Team);
    }

    #[test]
    fn tab_cycles_team_all_allies_skipping_empty() {
        assert_eq!(next_channel(Channel::Team, true, true), Channel::All);
        assert_eq!(next_channel(Channel::All, true, true), Channel::Allies);
        assert_eq!(next_channel(Channel::Allies, true, true), Channel::Team);
        assert_eq!(next_channel(Channel::Team, true, false), Channel::All);
        assert_eq!(next_channel(Channel::All, true, false), Channel::Team);
        assert_eq!(next_channel(Channel::All, false, true), Channel::Allies);
        assert_eq!(next_channel(Channel::Allies, false, true), Channel::All);
        assert_eq!(next_channel(Channel::All, false, false), Channel::All);
    }

    #[test]
    fn masks_need_an_ally_in_the_match_and_for_team_the_same_lobby_team() {
        let mut in_match = [false; 8];
        let mut allied = [false; 8];
        let mut team = [false; 8];
        in_match[0] = true;
        in_match[1] = true;
        in_match[2] = true;
        in_match[3] = false; // empty slot
        allied[0] = true; // self: never counted
        allied[1] = true;
        allied[2] = true;
        allied[3] = true;
        team[1] = true;
        assert_eq!(recipient_mask(0, &in_match, &allied, None), 0b0110);
        assert_eq!(recipient_mask(0, &in_match, &allied, Some(&team)), 0b0010);
        assert_eq!(recipient_mask(8, &in_match, &allied, None), 0);
    }
}
