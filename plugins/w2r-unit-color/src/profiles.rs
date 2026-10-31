//! `color_profiles.json`: the player's color profiles, in the warcraft2.site / W2 Toolkit preset
//! shape with a name:
//!
//! ```json
//! {
//!   "profiles": [
//!     {
//!       "name": "Classic Colors + Black 2 Pink",
//!       "colors": {
//!         "players": [
//!           { "player": 1, "color": "#A40000" },
//!           { "player": 6, "color": "#FF00FF" }
//!         ],
//!         "selectionHighlight": "#00FF00"
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! A player left out keeps the game's own color. The `colors` object's other fields (highlights,
//! bars) are not applied, but are kept when the file is saved again; an `author` from a site preset
//! is dropped. Reading also accepts a bare list of profiles, the site's listing
//! (`{"items": [...]}`), one profile, or just a `colors` object, so a preset copied from the site or
//! W2 Toolkit can be pasted in as it is.

use crate::json::{self, Value};
use crate::model::{self, MAX_PROFILES, PLAYERS, Profile};

/// Why one profile in the file was skipped (`entry` counts from 1); the rest still load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub entry: usize,
    pub what: &'static str,
}

/// Every profile in the file, in file order, with what was skipped. `Err` when the file is not
/// JSON at all or has no list of profiles; then nothing may be written over it.
pub fn parse(text: &str) -> Result<(Vec<Profile>, Vec<Warning>), &'static str> {
    let doc = json::parse(text).map_err(|code| match code {
        "duplicate_key" => "not valid JSON (a key appears twice in one object)",
        _ => "not valid JSON",
    })?;
    let list = doc.get("profiles").or_else(|| doc.get("items"));
    let items: Vec<&Value> = match (list, &doc) {
        (Some(Value::Array(items)), _) => items.iter().collect(),
        (Some(_), _) => return Err("\"profiles\" is not a list"),
        (None, Value::Array(items)) => items.iter().collect(),
        (None, Value::Object(_)) => vec![&doc],
        (None, _) => return Err("expected a list of profiles"),
    };
    let mut out: Vec<Profile> = Vec::new();
    let mut warnings = Vec::new();
    for (n, item) in items.into_iter().enumerate() {
        let entry = n + 1;
        let p = match profile_of(item, entry) {
            Ok(p) => p,
            Err(what) => {
                warnings.push(Warning { entry, what });
                continue;
            }
        };
        if out.iter().any(|o| o.name.eq_ignore_ascii_case(&p.name)) {
            warnings.push(Warning {
                entry,
                what: "a profile with this name came earlier",
            });
        } else if out.len() >= MAX_PROFILES {
            warnings.push(Warning {
                entry,
                what: "too many profiles",
            });
        } else {
            out.push(p);
        }
    }
    Ok((out, warnings))
}

/// Keys of a profile that are not kept as extras when the colors sit at the profile's own level.
const OWN_KEYS: [&str; 3] = ["players", "name", "author"];

fn profile_of(item: &Value, entry: usize) -> Result<Profile, &'static str> {
    let Value::Object(_) = item else {
        return Err("not an object");
    };
    // `{"name", "colors": {...}}`, or the colors object itself.
    let colors = match item.get("colors") {
        Some(c @ Value::Object(_)) => c,
        Some(_) => return Err("\"colors\" is not an object"),
        None => item,
    };
    let players = match colors.get("players") {
        Some(Value::Array(players)) => players,
        Some(_) => return Err("\"players\" is not a list"),
        None => return Err("no \"players\" list"),
    };
    let mut out = [None; PLAYERS];
    for (n, entry) in players.iter().enumerate() {
        let (slot, color) = match entry {
            // Eight colors in player order.
            Value::String(s) => (n, s.as_str()),
            Value::Object(_) => {
                let number = entry
                    .get("player")
                    .and_then(Value::as_number)
                    .ok_or("a player has no number")?;
                let p = number
                    .as_u64()
                    .filter(|p| (1..=PLAYERS as u64).contains(p))
                    .ok_or("a player number is not 1 to 8")?;
                let color = entry
                    .get("color")
                    .and_then(Value::as_str)
                    .ok_or("a player has no color")?;
                (p as usize - 1, color)
            }
            _ => return Err("a player is not an object"),
        };
        if slot >= PLAYERS {
            return Err("more than 8 players");
        }
        out[slot] = model::parse_entry(color).map_err(|_| "a color is not #RRGGBB")?;
    }
    let text = |k| item.get(k).and_then(Value::as_str).unwrap_or("");
    let extra = match colors {
        Value::Object(fields) => fields
            .iter()
            .filter(|(k, _)| !OWN_KEYS.contains(&k.as_str()))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    Ok(Profile {
        name: model::clean_name(text("name"), &format!("Profile {entry}")),
        colors: out,
        extra,
    })
}

/// The whole file for `profiles`. Players with the game's own color are left out.
pub fn write(profiles: &[Profile]) -> String {
    let list = profiles.iter().map(to_value).collect();
    json::pretty(&Value::Object(vec![(
        "profiles".into(),
        Value::Array(list),
    )]))
}

fn to_value(p: &Profile) -> Value {
    let players = p
        .colors
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let rgb = (*c)?;
            Some(Value::Object(vec![
                ("player".into(), json::int((i + 1) as u64)),
                ("color".into(), Value::String(model::to_hex(rgb))),
            ]))
        })
        .collect();
    let mut colors = vec![("players".to_string(), Value::Array(players))];
    colors.extend(p.extra.iter().cloned());
    Value::Object(vec![
        ("name".into(), Value::String(p.name.clone())),
        ("colors".into(), Value::Object(colors)),
    ])
}

/// Takes the profile named `name` (ignoring case and surrounding spaces) out of `profiles`.
pub fn remove(profiles: &mut Vec<Profile>, name: &str) -> Option<Profile> {
    let name = name.trim();
    let i = profiles
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name))?;
    Some(profiles.remove(i))
}

/// `profiles` with `p` added, or replacing the profile of the same name (ignoring case).
pub fn upsert(profiles: &mut Vec<Profile>, p: Profile) -> Result<(), &'static str> {
    if let Some(old) = profiles
        .iter_mut()
        .find(|o| o.name.eq_ignore_ascii_case(&p.name))
    {
        *old = p;
        Ok(())
    } else if profiles.len() >= MAX_PROFILES {
        Err("too many profiles")
    } else {
        profiles.push(p);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BLACK, CLASSIC, default_profiles};

    /// The maintainer's example: one preset's `colors` object, as W2 Toolkit exports it.
    const TOOLKIT: &str = r##"{
      "players": [
        { "player": 1, "color": "#A60000" }, { "player": 2, "color": "#003CC0" },
        { "player": 3, "color": "#2DB696" }, { "player": 4, "color": "#643296" },
        { "player": 5, "color": "#FB8E14" }, { "player": 6, "color": "#FF00FF" },
        { "player": 7, "color": "#E3E3E3" }, { "player": 8, "color": "#FFFF49" }
      ],
      "selectionHighlight": "#00FF00", "enemySelectionHighlight": "#FF0000",
      "allyHighlight": "#FFFF00", "critterHighlight": "#A2A2A6",
      "goldMineHighlight": "#694114", "oilPatchHighlight": "#FFFBF3",
      "hpBarHealthy": "#347004", "hpBarDamaged": "#FCFC00",
      "hpBarCritical": "#FC0000", "manaBar": "#3470A4"
    }"##;

    #[test]
    fn reads_a_bare_toolkit_preset_and_keeps_its_extras() {
        let (p, w) = parse(TOOLKIT).unwrap();
        assert!(w.is_empty());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Profile 1");
        assert_eq!(p[0].colors[BLACK], Some([0xff, 0, 0xff]));
        assert!(p[0].colors.iter().all(Option::is_some));
        assert_eq!(p[0].extra.len(), 10);
        assert_eq!(p[0].extra[0].0, "selectionHighlight");

        // Saved and read again: nothing lost, and the extras sit inside "colors".
        let text = write(&p);
        let (back, _) = parse(&text).unwrap();
        assert_eq!(back, p);
        assert!(text.contains("\"manaBar\": \"#3470A4\""));
    }

    #[test]
    fn written_files_read_back() {
        let profiles = default_profiles();
        let text = write(&profiles);
        let (back, warnings) = parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, profiles);
        assert!(text.starts_with("{\n  \"profiles\": [\n    {\n      \"name\": \"Classic\","));
        // Black to pink lists black only; the rest keep the game's colors.
        let pink = &back[2];
        assert_eq!(pink.colors.iter().filter(|c| c.is_some()).count(), 1);
    }

    #[test]
    fn reads_the_site_listing_and_named_profiles() {
        let site = r##"{"items":[{"id":"x","name":"Classic Colors + Black 2 Pink","author":"Maxx444#2154",
            "colors":{"players":[{"color":"#A40000","player":1},{"color":"#FF00FF","player":6}]}}],
            "hasMore":false}"##;
        let (p, _) = parse(site).unwrap();
        assert_eq!(p[0].name, "Classic Colors + Black 2 Pink");
        assert_eq!(p[0].colors[0], Some(CLASSIC[0]));
        assert_eq!(p[0].colors[1], None);

        let list = r##"[{"name":"A","colors":{"players":["#010203","#040506"]}}]"##;
        let (p, _) = parse(list).unwrap();
        assert_eq!(p[0].colors[1], Some([4, 5, 6]));
    }

    #[test]
    fn bad_profiles_are_skipped_with_their_number() {
        let text = r##"{"profiles": [
            {"name": "Good", "colors": {"players": [{"player": 1, "color": "#000000"}]}},
            {"name": "No players", "colors": {}},
            {"name": "Bad number", "colors": {"players": [{"player": 9, "color": "#000000"}]}},
            {"name": "Bad color", "colors": {"players": [{"player": 1, "color": "black"}]}},
            {"name": "good", "colors": {"players": []}},
            "text"
        ]}"##;
        let (p, w) = parse(text).unwrap();
        assert_eq!(p.len(), 1);
        let entries: Vec<usize> = w.iter().map(|w| w.entry).collect();
        assert_eq!(entries, vec![2, 3, 4, 5, 6]);
        assert_eq!(w[1].what, "a player number is not 1 to 8");
    }

    #[test]
    fn a_broken_file_is_an_error() {
        assert!(parse("").is_err());
        assert!(parse("[profile]").is_err());
        assert!(parse(r#"{"profiles": 3}"#).is_err());
        assert!(parse("42").is_err());
    }

    #[test]
    fn caps_the_number_of_profiles() {
        let one = r##"{"colors":{"players":[]}}"##;
        let names: Vec<String> = (0..MAX_PROFILES + 3)
            .map(|i| one.replacen('{', &format!("{{\"name\":\"p{i}\","), 1))
            .collect();
        let (p, w) = parse(&format!("[{}]", names.join(","))).unwrap();
        assert_eq!(p.len(), MAX_PROFILES);
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn a_new_file_is_maxx444s_color_profiles_json() {
        // Byte for byte the file Maxx444 uses (2026-10-30), so a fresh install starts with it.
        let expected = include_str!("../tests/color_profiles.json");
        assert_eq!(write(&default_profiles()), expected);
    }

    #[test]
    fn remove_takes_one_profile_by_name() {
        let mut list = default_profiles();
        let n = list.len();
        assert_eq!(
            remove(&mut list, "  classic ").map(|p| p.name),
            Some("Classic".into())
        );
        assert_eq!(list.len(), n - 1);
        assert!(list.iter().all(|p| p.name != "Classic"));
        assert_eq!(remove(&mut list, "Classic"), None);
        assert_eq!(remove(&mut list, ""), None);
    }

    #[test]
    fn upsert_replaces_by_name() {
        let mut list = default_profiles();
        let n = list.len();
        let mut p = list[0].clone();
        p.name = p.name.to_uppercase();
        p.colors[0] = Some([1, 2, 3]);
        upsert(&mut list, p).unwrap();
        assert_eq!(list.len(), n);
        assert_eq!(list[0].colors[0], Some([1, 2, 3]));
        let q = Profile {
            name: "New".into(),
            colors: [None; PLAYERS],
            extra: Vec::new(),
        };
        upsert(&mut list, q).unwrap();
        assert_eq!(list.len(), n + 1);
    }
}
