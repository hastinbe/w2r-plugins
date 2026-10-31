//! Allied Minimap Colors: allies blue and enemies red on the minimap, your own units unchanged.
//! The idea and the method come from Lambchops' "Allied Colors" plugin (`allied_color.w2p`) for the
//! Warcraft II Battle.net Edition Combat Edition: it rewrote the game's eight-byte per-player
//! minimap colour table from the alliance table and toggled with a key. Remastered still has that
//! table. Pure rules only, tested on any machine; the game-facing half is in `plugin`.

/// Number of player slots in the minimap colour table.
pub const SLOTS: usize = 8;

/// The game's stock minimap colours by colour index (red .. yellow): render palette entries.
/// The game copies `STOCK[colour of player p]` into the live table when a match starts. Seeing
/// exactly these bytes is also how the plugin checks it found the table (build 1.0.2.2818).
pub const STOCK: [u8; SLOTS] = [0xd0, 0x01, 0xd8, 0xdc, 0xe0, 0xe4, 0xff, 0x02];

/// Palette entry for an ally's dots: the stock blue dot.
pub const ALLY: u8 = 0x01;
/// Palette entry for an enemy's dots: the stock red dot (the first entry of the red ramp, so a
/// custom Player 1 (Red) color changes it too).
pub const ENEMY: u8 = 0xd0;

/// `DiplomacyStance` is indexed `[player * 16 + other]` (as the host's minimap ping reads it).
pub const STANCE_STRIDE: usize = 16;

/// Settings keys.
pub const ENABLED_KEY: &str = "allied_minimap";
/// (`allied_hotkey` in 0.2.0 held an index into a longer list; a new key so it is not misread.)
pub const HOTKEY_KEY: &str = "allied_key";

/// The toggle key choices, in dropdown order, with their Windows virtual-key codes.
/// F2, F3 and F4 also jump the camera to a saved location in Remastered (the game sees the key
/// too); F6, F7 and Insert are left out because the game uses them.
pub const HOTKEYS: [(&str, i32); 5] = [
    ("F1", 0x70),
    ("F8", 0x77),
    ("F2 (also moves the camera)", 0x71),
    ("F3 (also moves the camera)", 0x72),
    ("F4 (also moves the camera)", 0x73),
];

/// F1, the default.
pub const DEFAULT_HOTKEY: i64 = 0;

/// The options screen's texts for the checkbox and the hotkey (the host refuses a setting whose
/// description is over 256 bytes or label over 64).
pub const ENABLED_LABEL: &str = "Allied Minimap Colors";
pub const ENABLED_DESCRIPTION: &str = "On the minimap, allies are blue and enemies red; your own units keep their color. The hotkey below turns it on and off in a match. Inspired by Lambchops' Allied Colors for the Battle.net Edition.";
pub const HOTKEY_LABEL: &str = "Allied Minimap Colors Hotkey";
pub const HOTKEY_DESCRIPTION: &str = "Press it in a match to turn Allied Minimap Colors on or off (not while chatting). Default F1. F2, F3 and F4 also jump the camera to a saved location, as the game uses them too.";

/// The hotkey choice stored as `index`: its label and virtual-key code. Out of range is F1.
pub fn hotkey(index: i64) -> (&'static str, i32) {
    usize::try_from(index)
        .ok()
        .and_then(|i| HOTKEYS.get(i))
        .copied()
        .unwrap_or(HOTKEYS[DEFAULT_HOTKEY as usize])
}

/// The live table the game uses outside Allied Minimap Colors: each player's stock dot by the
/// colour the match gave them (`reorder[p]`, `TeamColourReorder`). A colour index out of range
/// keeps what is there now.
pub fn stock_table(reorder: &[u8; SLOTS], current: &[u8; SLOTS]) -> [u8; SLOTS] {
    let mut out = *current;
    for (p, slot) in out.iter_mut().enumerate() {
        if let Some(&dot) = STOCK.get(usize::from(reorder[p])) {
            *slot = dot;
        }
    }
    out
}

/// The dot of team colour `c`: the first entry of its palette ramp when the colour is recoloured
/// and its stock dot is not already that entry (Blue's dot is the shared entry 1 and White's 255,
/// which the recolour cannot touch; their ramps, 0xd4 and 0xe8, it rewrites). Otherwise the stock
/// dot. `ramp` is the colour's ramp start, if it has one (Yellow has none).
pub fn dot_for(c: usize, recolored: bool, ramp: Option<u8>) -> Option<u8> {
    let stock = *STOCK.get(c)?;
    Some(match ramp {
        Some(first) if recolored && first != stock => first,
        _ => stock,
    })
}

/// The live table with the plugin's colours and Allied Minimap Colors off: each player's dot by
/// the colour the match gave them (`reorder`), [`dot_for`] each. A colour index out of range keeps
/// what is there now.
pub fn base_table(
    reorder: &[u8; SLOTS],
    current: &[u8; SLOTS],
    recolored: &[bool; SLOTS],
    ramps: &[Option<u8>; SLOTS],
) -> [u8; SLOTS] {
    let mut out = *current;
    for (p, slot) in out.iter_mut().enumerate() {
        let c = usize::from(reorder[p]);
        if let Some(dot) = dot_for(
            c,
            recolored.get(c) == Some(&true),
            ramps.get(c).copied().flatten(),
        ) {
            *slot = dot;
        }
    }
    out
}

/// The live table with Allied Minimap Colors on. `stance_row` is the local player's row of
/// `DiplomacyStance` (non-zero: allied). The local player keeps their dot from `base`; `None` when
/// the local player is not one of the eight (nothing to colour against).
pub fn allied_table(
    local: u8,
    stance_row: &[u8; SLOTS],
    base: &[u8; SLOTS],
) -> Option<[u8; SLOTS]> {
    let local = usize::from(local);
    if local >= SLOTS {
        return None;
    }
    let mut out = [0u8; SLOTS];
    for (p, slot) in out.iter_mut().enumerate() {
        *slot = if p == local {
            base[p]
        } else if stance_row[p] != 0 {
            ALLY
        } else {
            ENEMY
        };
    }
    Some(out)
}

/// What one frame of the toggle key amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Nothing new: the key is up, or still held from before.
    None,
    /// The key went down and may toggle.
    Toggle,
    /// The key went down, but not while armed (chat box open, game window in the background).
    Ignored,
}

/// Turns a key held down into one press: only the frame it goes down counts. A press while not
/// armed is `Ignored`, and holding the key until armed again does not toggle later.
#[derive(Debug, Default, Clone, Copy)]
pub struct Toggle {
    was_down: bool,
}

impl Toggle {
    pub const fn new() -> Self {
        Self { was_down: false }
    }

    pub fn press(&mut self, down: bool, armed: bool) -> Press {
        let edge = down && !self.was_down;
        self.was_down = down;
        match (edge, armed) {
            (false, _) => Press::None,
            (true, true) => Press::Toggle,
            (true, false) => Press::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [u8; SLOTS] = [0, 1, 2, 3, 4, 5, 6, 7];

    #[test]
    fn stock_follows_the_colour_each_player_got() {
        assert_eq!(stock_table(&IDENTITY, &[0; SLOTS]), STOCK);
        let reorder = [1, 0, 2, 3, 4, 5, 6, 7];
        let t = stock_table(&reorder, &[0; SLOTS]);
        assert_eq!(t[0], 0x01);
        assert_eq!(t[1], 0xd0);
        // An unknown colour keeps what is there.
        let t = stock_table(&[9, 0, 0, 0, 0, 0, 0, 0], &[0x55; SLOTS]);
        assert_eq!(t[0], 0x55);
        assert_eq!(t[1], 0xd0);
    }

    #[test]
    fn allies_blue_enemies_red_self_unchanged() {
        // Player 2 (index 2) is local; allied with 0 and 5.
        let mut row = [0u8; SLOTS];
        row[0] = 1;
        row[5] = 1;
        let t = allied_table(2, &row, &STOCK).unwrap();
        assert_eq!(t, [ALLY, ENEMY, 0xd8, ENEMY, ENEMY, ALLY, ENEMY, ENEMY]);
    }

    #[test]
    fn self_keeps_its_own_dot_even_if_the_stance_row_says_allied() {
        let row = [1u8; SLOTS];
        let t = allied_table(0, &row, &STOCK).unwrap();
        assert_eq!(t[0], 0xd0);
        assert!(t[1..].iter().all(|&d| d == ALLY));
    }

    #[test]
    fn no_local_player_no_table() {
        assert_eq!(allied_table(8, &[0; SLOTS], &STOCK), None);
        assert_eq!(allied_table(255, &[0; SLOTS], &STOCK), None);
    }

    /// The ramps as the plugin has them: red to white, four entries apart from 0xd0.
    const RAMPS: [Option<u8>; SLOTS] = [
        Some(0xd0),
        Some(0xd4),
        Some(0xd8),
        Some(0xdc),
        Some(0xe0),
        Some(0xe4),
        Some(0xe8),
        None,
    ];

    #[test]
    fn recolored_blue_and_white_dots_move_to_their_ramps() {
        let mut recolored = [false; SLOTS];
        assert_eq!(base_table(&IDENTITY, &STOCK, &recolored, &RAMPS), STOCK);
        recolored[1] = true;
        recolored[6] = true;
        recolored[7] = true; // yellow: no ramp, stays
        recolored[0] = true; // red: its dot already is its ramp's first entry
        let t = base_table(&IDENTITY, &STOCK, &recolored, &RAMPS);
        assert_eq!(t, [0xd0, 0xd4, 0xd8, 0xdc, 0xe0, 0xe4, 0xe8, 0x02]);
    }

    #[test]
    fn the_dot_follows_the_colour_the_player_got() {
        // Player 0 plays White, player 6 plays Red.
        let mut reorder = IDENTITY;
        reorder[0] = 6;
        reorder[6] = 0;
        let mut recolored = [false; SLOTS];
        recolored[6] = true;
        let t = base_table(&reorder, &[0; SLOTS], &recolored, &RAMPS);
        assert_eq!(t[0], 0xe8);
        assert_eq!(t[6], 0xd0);
    }

    #[test]
    fn allied_colors_keep_your_recolored_dot() {
        let mut recolored = [false; SLOTS];
        recolored[6] = true;
        let base = base_table(&IDENTITY, &STOCK, &recolored, &RAMPS);
        let t = allied_table(6, &[0; SLOTS], &base).unwrap();
        assert_eq!(t[6], 0xe8);
        assert!(t.iter().enumerate().all(|(p, &d)| p == 6 || d == ENEMY));
    }

    #[test]
    fn hotkey_defaults_to_f1() {
        assert_eq!(hotkey(DEFAULT_HOTKEY), ("F1", 0x70));
        assert_eq!(hotkey(-1).1, 0x70);
        assert_eq!(hotkey(99).1, 0x70);
        assert_eq!(hotkey(1).1, 0x77);
        assert_eq!(hotkey(2).1, 0x71);
        assert_eq!(HOTKEYS.map(|(_, vk)| vk), [0x70, 0x77, 0x71, 0x72, 0x73]);
        // F6, F7 and Insert are used by the game.
        assert!(
            HOTKEYS
                .iter()
                .all(|&(_, vk)| vk != 0x75 && vk != 0x76 && vk != 0x2d)
        );
        // The host takes at most 16 choices of at most 64 bytes.
        assert!(HOTKEYS.len() >= 2 && HOTKEYS.len() <= 16);
        assert!(HOTKEYS.iter().all(|(l, _)| l.len() <= 64));
    }

    #[test]
    fn toggle_fires_once_per_press() {
        let mut t = Toggle::default();
        assert_eq!(t.press(false, true), Press::None);
        assert_eq!(t.press(true, true), Press::Toggle);
        assert_eq!(t.press(true, true), Press::None); // held
        assert_eq!(t.press(true, true), Press::None); // the second view's call in the same frame
        assert_eq!(t.press(false, true), Press::None);
        assert_eq!(t.press(true, true), Press::Toggle);
    }

    #[test]
    fn toggle_ignores_a_key_pressed_while_not_armed() {
        let mut t = Toggle::default();
        assert_eq!(t.press(true, false), Press::Ignored); // typing F2 in chat
        assert_eq!(t.press(true, true), Press::None); // chat closed, key still down
        assert_eq!(t.press(false, true), Press::None);
        assert_eq!(t.press(true, true), Press::Toggle);
    }

    #[test]
    fn options_texts_fit_the_hosts_limits() {
        for (label, description) in [
            (ENABLED_LABEL, ENABLED_DESCRIPTION),
            (HOTKEY_LABEL, HOTKEY_DESCRIPTION),
        ] {
            assert!(label.len() <= 64, "{label}");
            assert!(description.len() <= 256, "{label}: {}", description.len());
        }
    }
}
