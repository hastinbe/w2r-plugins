//! Pure recolor rules: the eight team colours, colour profiles, and what the settings store. No
//! host calls, so everything here is tested on any machine. Ramp shading follows wc2r-plus `recolour.cpp`.

/// An sRGB colour.
pub type Rgb = [u8; 3];

/// Number of team colours (colour index 0 red .. 7 yellow).
pub const PLAYERS: usize = 8;
/// The black team's colour index.
pub const BLACK: usize = 5;
/// Length of one team ramp in the render palette (first entry is the base shade).
pub const RAMP: usize = 4;
/// Render palette entry where the red ramp starts; the following ramps are four entries apart.
const FIRST_RAMP: usize = 0xd0;

/// One choice per colour: `None` leaves the game's own colour alone.
pub type Colors = [Option<Rgb>; PLAYERS];

/// Colour names in colour-index order, as the game's lobby shows them.
pub const NAMES: [&str; PLAYERS] = [
    "Red", "Blue", "Teal", "Violet", "Orange", "Black", "White", "Yellow",
];

/// Settings key of colour `i` (`color1` .. `color8`); the value is `#RRGGBB` or empty.
pub fn key(i: usize) -> String {
    format!("color{}", i + 1)
}

/// wc2r-plus's Black2Pink and Black2Cyan.
pub const PINK: Rgb = [0xe8, 0x7b, 0xdd];
pub const CYAN: Rgb = [0x00, 0xc6, 0xe0];
/// The pink of the built-in pink profiles (as in Maxx444's color_profiles.json).
pub const MAGENTA: Rgb = [0xff, 0x00, 0xff];
/// The violet of Maxx444's profiles.
pub const MAXX_VIOLET: Rgb = [0x6a, 0x0d, 0xad];

/// Classic Warcraft II colours (the DOS / Battle.net Edition palette).
pub const CLASSIC: [Rgb; PLAYERS] = [
    [0xa4, 0x00, 0x00],
    [0x00, 0x3c, 0xc0],
    [0x2c, 0xb4, 0x94],
    [0x98, 0x48, 0xb0],
    [0xf0, 0x84, 0x14],
    [0x28, 0x28, 0x3c],
    [0xe0, 0xe0, 0xe0],
    [0xfc, 0xfc, 0x48],
];

/// The Remastered colours, written out (choosing them pins the values; "Game default" does not).
pub const REMASTERED: [Rgb; PLAYERS] = [
    [0xa6, 0x00, 0x00],
    [0x00, 0x96, 0xff],
    [0x2d, 0xb6, 0x96],
    [0x9a, 0x49, 0xb2],
    [0xfb, 0x8e, 0x14],
    [0x28, 0x28, 0x3d],
    [0xe3, 0xe3, 0xe3],
    [0xff, 0xff, 0x49],
];

/// A named set of colours, with who made it.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub colors: Colors,
    /// The `colors` object's other fields (highlight and bar colors from warcraft2.site and W2
    /// Toolkit presets). Not applied, only kept so the file loses nothing when it is saved again.
    pub extra: Vec<(String, crate::json::Value)>,
}

/// What a new `color_profiles.json` holds, in this order.
pub fn default_profiles() -> Vec<Profile> {
    let all = |c: [Rgb; PLAYERS]| c.map(Some);
    let black = |rgb| {
        let mut c = [None; PLAYERS];
        c[BLACK] = Some(rgb);
        c
    };
    let p = |name: &str, colors| Profile {
        name: name.into(),
        colors,
        extra: Vec::new(),
    };
    // Maxx444's: a deeper violet, with black as pink or cyan.
    let maxx = |rgb| {
        let mut c = [None; PLAYERS];
        c[3] = Some(MAXX_VIOLET);
        c[BLACK] = Some(rgb);
        c
    };
    vec![
        p("Classic", all(CLASSIC)),
        p("Remastered", all(REMASTERED)),
        p("Black to pink", black(MAGENTA)),
        p("Black to cyan", black(CYAN)),
        p("Maxx444's + Pink", maxx(MAGENTA)),
        p("Maxx444's + Cyan", maxx(CYAN)),
    ]
}

/// The options screen's profile dropdown: `Custom`, `Game default`, then the profiles as they
/// were when the game started (the host fixes a dropdown's list at load).
pub const CHOICE_CUSTOM: i64 = 0;
pub const CHOICE_GAME_DEFAULT: i64 = 1;
/// Most entries a dropdown may have (`W2R_MAX_SETTING_CHOICE_COUNT`), and so most profiles listed.
pub const MAX_CHOICES: usize = 16;
pub const MAX_LISTED: usize = MAX_CHOICES - 2;
/// Longest dropdown label the host accepts.
const MAX_CHOICE_LABEL: usize = 64;
/// Profile names that cannot be saved over or deleted: the dropdown's Game default and the two
/// built-in color sets.
pub const PROTECTED: [&str; 3] = ["Game default", "Classic", "Remastered"];

/// Whether `name` (spaces around it and case ignored) is one of [`PROTECTED`].
pub fn is_protected(name: &str) -> bool {
    let name = name.trim();
    PROTECTED.iter().any(|p| p.eq_ignore_ascii_case(name))
}

/// Settings key of the dropdown (an index).
pub const PROFILE_KEY: &str = "profile";
/// Settings keys of the options screen's save fields: the checkbox that makes Apply save, and the
/// name it saves under.
pub const SAVE_KEY: &str = "save_profile";
pub const NAME_KEY: &str = "profile_name";
/// Settings keys of the options screen's delete: the checkbox that asks for it, and the second one
/// that confirms it. Apply deletes only when both are checked.
pub const DELETE_KEY: &str = "delete_profile";
pub const CONFIRM_KEY: &str = "delete_profile_confirm";

/// What Apply on the options screen does with the save and delete checkboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAction {
    None,
    Save,
    Delete,
    /// Delete was checked without "Yes, Delete It".
    DeleteNotConfirmed,
    /// Save and Delete were both checked: nothing is done.
    Both,
}

pub fn profile_action(save: bool, delete: bool, confirmed: bool) -> ProfileAction {
    match (save, delete, confirmed) {
        (true, true, _) => ProfileAction::Both,
        (true, false, _) => ProfileAction::Save,
        (false, true, true) => ProfileAction::Delete,
        (false, true, false) => ProfileAction::DeleteNotConfirmed,
        // "Yes" alone does nothing.
        (false, false, _) => ProfileAction::None,
    }
}

/// The longest setting description and label the host accepts
/// (`W2R_MAX_SETTING_DESCRIPTION`, `W2R_MAX_SETTING_LABEL`); a longer one makes it refuse the whole
/// setting, which then is not on the options screen.
pub const MAX_DESCRIPTION: usize = 256;
pub const MAX_LABEL: usize = 64;

/// The options screen's texts for the profile name field and the save and delete boxes.
pub const NAME_LABEL: &str = "Save or Delete Profile Name";
pub const NAME_DESCRIPTION: &str = "The profile the boxes below save or delete. Custom when the colors match no profile. Saving as Custom, Game default, Classic or Remastered saves to Custom; another name saves over that profile or makes a new one.";
pub const SAVE_LABEL: &str = "Save Colors as Profile on Apply";
pub const SAVE_DESCRIPTION: &str = "Check, then press Apply: the colors below are saved to color_profiles.json under the name above, replacing a profile of the same name. A new name joins the Color Profile list at the next start.";
pub const DELETE_LABEL: &str = "Delete This Profile on Apply";
pub const DELETE_DESCRIPTION: &str = "Check, then also check Yes below and press Apply: the profile named above is removed from color_profiles.json. The colors in use stay. It leaves the Color Profile list at the next start.";
pub const CONFIRM_LABEL: &str = "Yes, Delete It (Cannot Be Undone)";
pub const CONFIRM_DESCRIPTION: &str =
    "Confirms the delete above. Without it, Apply deletes nothing.";

/// The description of team color `i`'s picker.
pub fn color_description(i: usize) -> String {
    format!(
        "The {} team color on your screen only. Pick the game's own color to leave it unchanged.",
        NAMES[i]
    )
}

/// The dropdown's entry and the name field's text when the colors match no profile, and the name
/// of the one saved profile that stands for it.
pub const CUSTOM_NAME: &str = "Custom";

/// Whether `name` is [`CUSTOM_NAME`], any case.
pub fn is_custom_name(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(CUSTOM_NAME)
}

/// The saved "Custom" profile, if there is one. It is not listed in the dropdown on its own:
/// the dropdown's single "Custom" stands for it.
pub fn custom_profile(profiles: &[Profile]) -> Option<&Profile> {
    profiles.iter().find(|p| is_custom_name(&p.name))
}

/// The profile names the dropdown lists after Custom and Game default, in file order, at most
/// [`MAX_LISTED`], leaving out the saved "Custom" profile.
pub fn listed_names(profiles: &[Profile]) -> Vec<String> {
    profiles
        .iter()
        .filter(|p| !is_custom_name(&p.name))
        .take(MAX_LISTED)
        .map(|p| p.name.clone())
        .collect()
}

/// The name a save goes under for `requested` (already cleaned): Custom and the built-in names
/// (Game default, Classic, Remastered) save to the one "Custom" profile; any other name saves over
/// that profile or makes a new one.
pub fn save_target(requested: &str) -> String {
    if is_custom_name(requested) || is_protected(requested) {
        CUSTOM_NAME.to_string()
    } else {
        requested.to_string()
    }
}

/// The dropdown's labels for `profiles`: Custom, Game default and [`listed_names`], cut to the
/// host's 64 bytes.
pub fn choice_labels(profiles: &[Profile]) -> Vec<String> {
    let mut out = vec![CUSTOM_NAME.to_string(), "Game default".to_string()];
    for name in listed_names(profiles) {
        let mut label = name;
        label.truncate(MAX_CHOICE_LABEL); // ASCII only (clean_name), so any cut is on a boundary
        out.push(label.trim_end().to_string());
    }
    out
}

/// The dropdown index that describes the colours in use: Game default when nothing is changed,
/// the listed profile (by name) that matches, or Custom.
pub fn choice_for(listed: &[String], all_default: bool, matched: Option<&str>) -> i64 {
    if all_default {
        return CHOICE_GAME_DEFAULT;
    }
    matched
        .and_then(|name| listed.iter().position(|n| n.eq_ignore_ascii_case(name)))
        .map_or(CHOICE_CUSTOM, |i| i as i64 + 2)
}

/// What choosing dropdown entry `index` asks for: `Some(None)` the game's own colours,
/// `Some(Some(name))` the profile listed at launch under that name, `None` nothing (Custom, or an
/// index out of range).
pub fn choice_target(listed: &[String], index: i64) -> Option<Option<&str>> {
    match index {
        CHOICE_GAME_DEFAULT => Some(None),
        i if i >= 2 => listed
            .get(usize::try_from(i - 2).ok()?)
            .map(|n| Some(n.as_str())),
        _ => None,
    }
}

/// What the options screen's profile name field should hold once the dropdown shows entry
/// `index`, given the name it holds now; `None` to leave it. `saved` is the profile in the file
/// whose colors are the ones in use, if any (one saved this session is in the file but not yet in
/// the dropdown, which is read at launch).
///
/// - A listed profile: its own name.
/// - Custom: "Custom", or the name of `saved`, so a name typed but not saved does not stay.
/// - Game default: empty when the field holds "Custom" or a profile's name; a new name typed for
///   saving is kept.
pub fn name_for_choice(
    listed: &[String],
    index: i64,
    current: &str,
    saved: Option<&str>,
) -> Option<String> {
    let want = match index {
        CHOICE_CUSTOM => saved.unwrap_or(CUSTOM_NAME).to_string(),
        _ => match choice_target(listed, index)? {
            Some(name) => name.to_string(),
            None => {
                let c = current.trim();
                let names_profile = is_custom_name(c)
                    || is_protected(c)
                    || listed.iter().any(|n| n.eq_ignore_ascii_case(c));
                if c.is_empty() || !names_profile {
                    return None;
                }
                String::new()
            }
        },
    };
    (current != want).then_some(want)
}

/// Parses `#RRGGBB` or `RRGGBB` (surrounding spaces allowed, any case).
pub fn parse_hex(text: &str) -> Option<Rgb> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `#RRGGBB`, upper case.
pub fn to_hex(rgb: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// An edit box or code entry that is neither a colour nor empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAColor;

/// What one colour edit box holds: empty means "game default", anything else must be a colour.
pub fn parse_entry(text: &str) -> Result<Option<Rgb>, NotAColor> {
    let t = text.trim();
    if t.is_empty() || t == "-" {
        return Ok(None);
    }
    parse_hex(t).map(Some).ok_or(NotAColor)
}

/// The settings value for a colour: six lowercase hex digits, the format the options screen's
/// colour picker stores (`W2R_SETTING_COLOR`).
pub fn to_setting(rgb: Rgb) -> String {
    format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// What a stored colour means: `None` (the game's own) when it is empty, unreadable, or equal to
/// the game's own colour `stock`, so picking the stock colour in the picker is "no override".
pub fn wanted(stored: &str, stock: Rgb) -> Option<Rgb> {
    parse_entry(stored)
        .ok()
        .flatten()
        .filter(|rgb| *rgb != stock)
}

/// The first profile whose colours are what is in use, treating a colour equal to the game's own
/// as unset on both sides. `None`: no profile matches (custom colours).
pub fn matching<'a>(
    profiles: &'a [Profile],
    cur: &Colors,
    stock: &[Rgb; PLAYERS],
) -> Option<&'a Profile> {
    let norm = |c: &Colors| -> Colors {
        let mut out = *c;
        for (o, s) in out.iter_mut().zip(stock) {
            if *o == Some(*s) {
                *o = None;
            }
        }
        out
    };
    let cur = norm(cur);
    profiles.iter().find(|p| norm(&p.colors) == cur)
}

/// The stock colours as read from the game, or the Remastered values when the read looks wrong
/// (all black, as an uninitialised table would be).
pub fn stock_or_default(read: [Rgb; PLAYERS]) -> [Rgb; PLAYERS] {
    if read.iter().all(|c| *c == [0, 0, 0]) {
        REMASTERED
    } else {
        read
    }
}

/// Most profiles kept from one file; the rest are ignored.
pub const MAX_PROFILES: usize = 64;
/// Longest profile name kept (in characters); longer ones are cut.
pub const MAX_NAME: usize = 40;

/// A name safe to show in the game's UI: printable ASCII, trimmed, at most `MAX_NAME`.
pub fn clean_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .take(MAX_NAME)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned.to_string()
    }
}

/// Render palette entry where colour `i`'s ramp starts, if it has one of its own. Red to white
/// use 0xd0..0xeb, four entries each (the game's unit remap table lists exactly these seven).
/// Yellow has no ramp of its own (0xec.. holds the gold mine highlight), so it is not touched.
pub fn ramp_base(i: usize) -> Option<usize> {
    (i < PLAYERS - 1).then_some(FIRST_RAMP + i * RAMP)
}

fn luma(c: Rgb) -> i32 {
    (c[0] as i32 * 299 + c[1] as i32 * 587 + c[2] as i32 * 114) / 1000
}

/// The ramp for `rgb`: the first entry is `rgb`, the rest keep the stock ramp's brightness ratio
/// to its first entry.
pub fn ramp(stock: &[Rgb; RAMP], rgb: Rgb) -> [Rgb; RAMP] {
    let base = luma(stock[0]);
    let mut out = [[0u8; 3]; RAMP];
    for k in 0..RAMP {
        for c in 0..3 {
            let mut v = rgb[c] as i32;
            if k > 0 && base > 0 {
                v = v * luma(stock[k]) / base;
            }
            out[k][c] = v.clamp(0, 255) as u8;
        }
    }
    out
}

/// The HD table entry (r, g, b as 0..1) for `rgb`.
pub fn float_rgb(rgb: Rgb) -> [f32; 3] {
    rgb.map(|c| c as f32 / 255.0)
}

/// The colour an HD table entry shows, rounded to bytes (values outside 0..1 are clamped).
pub fn rgb_of(entry: [f32; 3]) -> Rgb {
    entry.map(|f| (f.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(parse_hex("#A60000"), Some([0xa6, 0, 0]));
        assert_eq!(parse_hex(" 0096ff "), Some([0, 0x96, 0xff]));
        assert_eq!(to_hex([0, 0x96, 0xff]), "#0096FF");
        for bad in ["", "#", "#12345", "#1234567", "GG0000", "#12 345", "+12345"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn entries_allow_default() {
        assert_eq!(parse_entry(""), Ok(None));
        assert_eq!(parse_entry(" - "), Ok(None));
        assert_eq!(parse_entry("ff00ff"), Ok(Some([255, 0, 255])));
        assert_eq!(parse_entry("pink"), Err(NotAColor));
    }

    #[test]
    fn defaults_are_sane() {
        let d = default_profiles();
        assert!(d.iter().all(|p| p.name.len() <= MAX_NAME));
        assert_eq!(d[0].name, "Classic");
        assert_eq!(d.len(), 6);
        assert_eq!(d[2].colors[BLACK], Some(MAGENTA));
        assert_eq!(d[2].colors.iter().filter(|c| c.is_some()).count(), 1);
    }

    #[test]
    fn settings_values() {
        assert_eq!(to_setting([0xa4, 0, 0x3c]), "a4003c");
        assert_eq!(parse_entry("a4003c"), Ok(Some([0xa4, 0, 0x3c])));
        let stock = REMASTERED[0];
        assert_eq!(wanted("", stock), None);
        assert_eq!(wanted("junk", stock), None);
        assert_eq!(wanted(&to_setting(stock), stock), None);
        assert_eq!(wanted("#A40000", stock), Some([0xa4, 0, 0]));
    }

    #[test]
    fn finds_the_profile_in_use() {
        let profiles = default_profiles();
        let stock = REMASTERED;
        // The Remastered profile spells out the stock colours, so it matches "all unset".
        assert_eq!(
            matching(&profiles, &[None; PLAYERS], &stock).map(|p| p.name.as_str()),
            Some("Remastered")
        );
        assert_eq!(
            matching(&profiles, &CLASSIC.map(Some), &stock).map(|p| p.name.as_str()),
            Some("Classic")
        );
        let mut pink = [None; PLAYERS];
        pink[BLACK] = Some(MAGENTA);
        assert_eq!(
            matching(&profiles, &pink, &stock).map(|p| p.name.as_str()),
            Some("Black to pink")
        );
        pink[0] = Some([1, 2, 3]);
        assert_eq!(matching(&profiles, &pink, &stock), None);
    }

    #[test]
    fn stock_falls_back_when_unread() {
        assert_eq!(stock_or_default([[0; 3]; PLAYERS]), REMASTERED);
        assert_eq!(stock_or_default(CLASSIC), CLASSIC);
    }

    #[test]
    fn delete_needs_both_boxes_and_never_mixes_with_save() {
        use ProfileAction::*;
        assert_eq!(profile_action(false, false, false), None);
        assert_eq!(profile_action(false, false, true), None);
        assert_eq!(profile_action(true, false, false), Save);
        assert_eq!(profile_action(false, true, false), DeleteNotConfirmed);
        assert_eq!(profile_action(false, true, true), Delete);
        assert_eq!(profile_action(true, true, true), Both);
    }

    #[test]
    fn dropdown_labels_fit() {
        let mut d = default_profiles();
        d.push(Profile {
            name: "x".repeat(40),
            colors: [None; PLAYERS],
            extra: Vec::new(),
        });
        let labels = choice_labels(&d);
        assert_eq!(labels[0], "Custom");
        assert_eq!(labels[1], "Game default");
        assert_eq!(labels[2], "Classic");
        assert!(labels.iter().all(|l| l.len() <= 64 && !l.is_empty()));
        let many: Vec<Profile> = (0..30)
            .map(|i| Profile {
                name: format!("p{i}"),
                colors: [None; PLAYERS],
                extra: Vec::new(),
            })
            .collect();
        assert_eq!(choice_labels(&many).len(), MAX_CHOICES);
    }

    #[test]
    fn dropdown_index_round_trips() {
        let listed = vec!["Remastered".to_string(), "Classic".to_string()];
        assert_eq!(
            choice_for(&listed, true, Some("Remastered")),
            CHOICE_GAME_DEFAULT
        );
        assert_eq!(choice_for(&listed, false, Some("classic")), 3);
        assert_eq!(
            choice_for(&listed, false, Some("Added later")),
            CHOICE_CUSTOM
        );
        assert_eq!(choice_for(&listed, false, None), CHOICE_CUSTOM);
        assert_eq!(choice_target(&listed, 3), Some(Some("Classic")));
        assert_eq!(choice_target(&listed, CHOICE_GAME_DEFAULT), Some(None));
        assert_eq!(choice_target(&listed, CHOICE_CUSTOM), None);
        assert_eq!(choice_target(&listed, 9), None);
        assert_eq!(choice_target(&listed, -4), None);
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  ok  ", "f"), "ok");
        assert_eq!(clean_name("", "f"), "f");
        assert_eq!(clean_name("é\n", "f"), "??");
        assert_eq!(clean_name(&"x".repeat(100), "f").len(), MAX_NAME);
    }

    #[test]
    fn ramps_cover_red_to_white_only() {
        assert_eq!(ramp_base(0), Some(0xd0));
        assert_eq!(ramp_base(BLACK), Some(0xe4));
        assert_eq!(ramp_base(6), Some(0xe8));
        assert_eq!(ramp_base(7), None);
    }

    #[test]
    fn ramp_keeps_the_stock_shading() {
        let stock = [[40, 40, 40], [20, 20, 20], [60, 60, 60], [80, 80, 80]];
        let r = ramp(&stock, [200, 100, 50]);
        assert_eq!(r[0], [200, 100, 50]);
        assert_eq!(r[1], [100, 50, 25]);
        assert_eq!(r[2], [255, 150, 75]);
        assert_eq!(r[3], [255, 200, 100]);
    }

    #[test]
    fn black_base_leaves_the_ramp_flat() {
        let r = ramp(&[[0; 3]; RAMP], [10, 20, 30]);
        assert!(r.iter().all(|e| *e == [10, 20, 30]));
    }

    #[test]
    fn float_entries_round_trip() {
        for rgb in REMASTERED {
            assert_eq!(rgb_of(float_rgb(rgb)), rgb);
        }
        assert_eq!(rgb_of([-1.0, 2.0, 0.5]), [0, 255, 128]);
    }

    #[test]
    fn built_in_profile_names_are_protected() {
        for name in ["Game default", "classic", "  REMASTERED  ", "Classic"] {
            assert!(is_protected(name), "{name}");
        }
        for name in ["Classic 2", "My Classic", "Black to pink", "", "Custom"] {
            assert!(!is_protected(name), "{name}");
        }
    }
    #[test]
    fn the_name_field_follows_the_dropdown() {
        let listed = vec!["Classic".to_string(), "Mine".to_string()];
        let to = |i, cur, saved| name_for_choice(&listed, i, cur, saved);
        // Game default empties "Custom" or a profile's name, built in or listed, in any case...
        assert_eq!(
            to(CHOICE_GAME_DEFAULT, "Classic", None),
            Some(String::new())
        );
        assert_eq!(to(CHOICE_GAME_DEFAULT, " mine ", None), Some(String::new()));
        assert_eq!(
            to(CHOICE_GAME_DEFAULT, "Remastered", None),
            Some(String::new())
        );
        assert_eq!(to(CHOICE_GAME_DEFAULT, "custom", None), Some(String::new()));
        // ...but keeps a new name and leaves an empty field alone.
        assert_eq!(to(CHOICE_GAME_DEFAULT, "New one", None), None);
        assert_eq!(to(CHOICE_GAME_DEFAULT, "", None), None);
        // A listed profile puts its name there unless it is there already.
        assert_eq!(to(3, "Classic", None), Some("Mine".to_string()));
        assert_eq!(to(3, "Mine", None), None);
        // Custom shows "Custom", whatever was typed, unless the colors are a saved profile.
        assert_eq!(to(CHOICE_CUSTOM, "Mine", None), Some("Custom".to_string()));
        assert_eq!(to(CHOICE_CUSTOM, "Test", None), Some("Custom".to_string()));
        assert_eq!(to(CHOICE_CUSTOM, "Custom", None), None);
        assert_eq!(
            to(CHOICE_CUSTOM, "Custom", Some("Test")),
            Some("Test".to_string())
        );
        assert_eq!(to(CHOICE_CUSTOM, "Test", Some("Test")), None);
        // Out of range leaves it.
        assert_eq!(to(9, "Classic", None), None);
        assert!(is_custom_name(" CUSTOM ") && !is_custom_name("Customs"));
    }
    #[test]
    fn the_saved_custom_profile_is_the_dropdowns_one_custom() {
        let p = |n: &str| Profile {
            name: n.to_string(),
            colors: [None; PLAYERS],
            extra: Vec::new(),
        };
        let list = vec![p("Classic"), p("custom"), p("Mine")];
        assert_eq!(
            listed_names(&list),
            vec!["Classic".to_string(), "Mine".to_string()]
        );
        assert_eq!(
            choice_labels(&list),
            vec!["Custom", "Game default", "Classic", "Mine"]
        );
        assert_eq!(
            custom_profile(&list).map(|p| p.name.as_str()),
            Some("custom")
        );
        assert!(custom_profile(&list[..1]).is_none());
        // Custom and the built-in names save to Custom; other names as typed.
        for n in [
            "Custom",
            " custom ",
            "Game default",
            "classic",
            "Remastered",
        ] {
            assert_eq!(save_target(n), "Custom");
        }
        assert_eq!(save_target("Mine"), "Mine");
        assert_eq!(save_target("Custom 2"), "Custom 2");
    }

    #[test]
    fn options_texts_fit_the_hosts_limits() {
        let texts = [
            (NAME_LABEL, NAME_DESCRIPTION),
            (SAVE_LABEL, SAVE_DESCRIPTION),
            (DELETE_LABEL, DELETE_DESCRIPTION),
            (CONFIRM_LABEL, CONFIRM_DESCRIPTION),
        ];
        for (label, description) in texts {
            assert!(label.len() <= MAX_LABEL, "{label}");
            assert!(
                description.len() <= MAX_DESCRIPTION,
                "{label}: {}",
                description.len()
            );
        }
        for i in 0..PLAYERS {
            assert!(color_description(i).len() <= MAX_DESCRIPTION);
        }
    }
}
