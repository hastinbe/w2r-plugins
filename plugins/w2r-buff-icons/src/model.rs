//! Pure rules: where each spell icon goes, the seconds shown on it, and the health and mana rows
//! as the game draws them. No game access, so it is tested on any machine. Offsets are for game
//! build 1.0.2.2818 and were checked in the game with a read-only watcher.

/// Bytes in one unit record.
pub const RECORD: usize = 0x98;

/// `UnitTypeFlags` bits.
pub const TYPE_BUILDING: u32 = 0x20;
pub const TYPE_CASTER: u32 = 0x2_0000;

/// Unit record: hit points, and the mana byte (0 to 255) of a caster.
pub const HP: usize = 0x22;
const MANA: usize = 0x26;

/// Spell timers in game ticks (they fall by 1 a tick), and the longest each can be. `+0x4a` is one
/// signed counter: Haste above zero, Slow below.
const BLOODLUST: usize = 0x48;
const HASTE_SLOW: usize = 0x4a;
const INVISIBILITY: usize = 0x44;
const UNHOLY_ARMOR: usize = 0x46;
const FULL: [(usize, i32); 4] = [(BLOODLUST, 750), (HASTE_SLOW, 1000), (INVISIBILITY, 2000), (UNHOLY_ARMOR, 500)];
/// The spells with an icon, in the order the game draws them (`UnitBuffIcons`, 0x4efa30), with the
/// art slot of each (Haste or Slow by the counter's sign).
const ICONS: [(usize, usize); 4] = [(BLOODLUST, 0), (HASTE_SLOW, 1), (INVISIBILITY, 3), (UNHOLY_ARMOR, 4)];
const SLOW_ART: usize = 2;

/// An icon is 16 px square.
pub const ICON: i32 = 16;
/// The game's own x offset of each icon art slot (0x8c8db8): Bloodlust, Haste, Slow,
/// Invisibility, Unholy Armor. Fixed spots, so a lone Unholy Armor sits 48 px out.
pub const STOCK_ICON_X: [i32; 5] = [0, 16, 16, 32, 48];
/// How far a row of icons may run past the selection box: three icons (48 px) fit on a 43-px box
/// (Knight, Ogre, Paladin, Ogre Mage), two on a 32 to 40-px one.
const ICON_SLACK: i32 = 5;
/// Rows of icons are an icon plus 3 px apart.
pub const ICON_ROW_PITCH: i32 = ICON + 3;
/// Game ticks per second at the speed played (the timers fall about 50 a second).
pub const TICKS_PER_SECOND: i32 = 50;
/// The countdown shows only in a spell's last seconds.
pub const NUMBER_FROM: i32 = 10;

fn u8_at(r: &[u8], o: usize) -> u8 {
    r[o]
}
fn u16_at(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}
fn i16_at(r: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([r[o], r[o + 1]])
}

pub fn hp(r: &[u8]) -> u16 {
    u16_at(r, HP)
}
pub fn mana(r: &[u8]) -> u8 {
    u8_at(r, MANA)
}

fn ratio(n: f32, d: f32) -> f32 {
    if d > 0.0 { (n / d).clamp(0.0, 1.0) } else { 0.0 }
}

/// The art slot of each icon the game draws for this unit, in order (0 Bloodlust, 1 Haste, 2 Slow,
/// 3 Invisibility, 4 Unholy Armor). The game draws one for every timer that is not 0.
pub fn icon_arts(r: &[u8]) -> Vec<usize> {
    if r.len() < RECORD {
        return Vec::new();
    }
    ICONS
        .iter()
        .filter_map(|&(off, art)| match i16_at(r, off) {
            0 => None,
            v if off == HASTE_SLOW && v < 0 => Some(SLOW_ART),
            _ => Some(art),
        })
        .collect()
}

/// Seconds left on each icon that is in its last `NUMBER_FROM` seconds: (icon index, seconds).
/// The index is the icon's place among the unit's drawn icons (see `icon_offset`). A timer above
/// its spell's longest length (not a real cast) gets no number but still takes its place.
pub fn icon_numbers(r: &[u8]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    if r.len() < RECORD {
        return out;
    }
    let mut index = 0;
    for (off, _) in ICONS {
        let raw = i16_at(r, off);
        if raw == 0 {
            continue;
        }
        let ticks = if off == HASTE_SLOW { (raw as i32).abs() } else { u16_at(r, off) as i32 };
        let full = FULL.iter().find(|f| f.0 == off).map_or(0, |f| f.1);
        let secs = (ticks + TICKS_PER_SECOND - 1) / TICKS_PER_SECOND;
        if ticks <= full && secs <= NUMBER_FROM {
            out.push((index, secs));
        }
        index += 1;
    }
    out
}

/// Icons per row for a unit whose selection box is `box_w` (the game's table value, one less than
/// the pixels): 2 on most units, 3 on 43-px ones, 4 on ships, flyers and siege.
pub fn icons_per_row(box_w: i32) -> i32 {
    ((box_w + 1 + ICON_SLACK) / ICON).clamp(1, 4)
}

/// Where the icon at `index` goes, from the game's base point (the selection box's top-left, one
/// pixel out): packed from the left with no gaps, wrapping after `per_row`. `bottom` puts the
/// first row at the bottom of the box and stacks up (health bars below the unit, with Buff Icons
/// Follow Health Bars on); otherwise the first row is at the top and stacks down, like the game's
/// row. `box_h` is the game's table value.
pub fn icon_offset(index: i32, per_row: i32, box_h: i32, bottom: bool) -> (i32, i32) {
    let per_row = per_row.max(1);
    let (row, col) = (index / per_row, index % per_row);
    let dy = if bottom { box_h + 1 - ICON - row * ICON_ROW_PITCH } else { row * ICON_ROW_PITCH };
    (col * ICON, dy)
}

/// The see-through track behind each health and mana bar: black at 160 of 255.
pub const TRACK: u32 = 0xa000_0000;
/// Health and mana rows are 3 px.
pub const ROW: i32 = 3;

/// One of the game's own bars (health or mana), as the game would draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StockRow {
    pub y: i32,
    pub fill: f32,
    /// The palette entry the game colours it with.
    pub palette: u8,
}

/// `GameSettingsFlags` bits for the health bars.
pub const FLAG_SHOW_HEALTH: u32 = 0x20_0000;
pub const FLAG_ONLY_DAMAGED: u32 = 0x40_0000;
pub const FLAG_BELOW: u32 = 0x80_0000;
const MANA_FULL: u8 = 255;
/// Health colour by percent left, as the game's table at 0x8c8ca4: 75 % and up green, 50 % and up
/// yellow, below that red; mana blue.
pub const PALETTE_GREEN: u8 = 0x95;
pub const PALETTE_YELLOW: u8 = 0xfb;
pub const PALETTE_RED: u8 = 0xf9;
pub const PALETTE_MANA: u8 = 0xba;

pub fn health_palette(hp: u16, max_hp: u16) -> u8 {
    let pct = hp as u32 * 100 / max_hp.max(1) as u32;
    if pct >= 75 {
        PALETTE_GREEN
    } else if pct >= 50 {
        PALETTE_YELLOW
    } else {
        PALETTE_RED
    }
}

/// The health and mana rows the game's `DrawUnitHealthBar` (0x4efb00) would draw, with its own
/// rules: nothing unless health bars are shown; health always, or only when damaged; a caster's
/// mana under it (only when not full in the damaged-only case); above the box 16 px over it,
/// below it 9 px under it, where a unit without mana uses the second row. `top` is the selection
/// box's top and `h` its height (the game's table value).
pub fn stock_rows(top: i32, h: i32, flags: u32, caster: bool, hp: u16, max_hp: u16, mana: u8) -> Vec<StockRow> {
    let mut out = Vec::new();
    if flags & FLAG_SHOW_HEALTH == 0 {
        return out;
    }
    let below = flags & FLAG_BELOW != 0;
    let y = if below { top + h + 9 } else { top - 16 };
    let max_hp = max_hp.max(1);
    let always = flags & FLAG_ONLY_DAMAGED == 0 && hp < 10000 && max_hp < 10000;
    let health = always || hp != max_hp;
    if health {
        let dy = if below && !caster { ROW } else { 0 };
        out.push(StockRow { y: y + dy, fill: ratio(hp as f32, max_hp as f32), palette: health_palette(hp, max_hp) });
    }
    if caster && (always || mana != MANA_FULL) {
        let dy = if health || below { ROW } else { 0 };
        out.push(StockRow { y: y + dy, fill: ratio(mana as f32, MANA_FULL as f32), palette: PALETTE_MANA });
    }
    out
}

/// The filled width of a bar the way the game's painter cuts it: width times the fill, rounded
/// down to whole pixels.
pub fn stock_filled(width: i32, fill: f32) -> i32 {
    (width as f32 * fill.clamp(0.0, 1.0)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Vec<u8> {
        vec![0u8; RECORD]
    }
    fn put16(r: &mut [u8], o: usize, v: u16) {
        r[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn arts_follow_the_game_order_and_the_haste_slow_sign() {
        let mut r = rec();
        put16(&mut r, UNHOLY_ARMOR, 400);
        assert_eq!(icon_arts(&r), vec![4], "Unholy Armor alone");
        put16(&mut r, HASTE_SLOW, (-100i16) as u16);
        assert_eq!(icon_arts(&r), vec![2, 4], "Slow, then Unholy Armor");
        put16(&mut r, HASTE_SLOW, 100);
        put16(&mut r, BLOODLUST, 10);
        assert_eq!(icon_arts(&r), vec![0, 1, 4]);
    }

    #[test]
    fn icons_pack_left_and_wrap_by_unit_size() {
        // A lone Unholy Armor takes the first spot, not the game's 48 px out.
        assert_eq!(icon_offset(0, icons_per_row(31), 31, false), (0, 0));
        // Box widths from the game's table (pixels minus one).
        assert_eq!(icons_per_row(31), 2, "footman, grunt, peon");
        assert_eq!(icons_per_row(39), 2, "death knight");
        assert_eq!(icons_per_row(42), 3, "knight, ogre, paladin, ogre mage");
        assert_eq!(icons_per_row(63), 4, "ships, flyers, siege");
        assert_eq!(icons_per_row(71), 4, "never more than 4");
        // Two per row: the third icon starts a row an icon plus 3 px lower.
        assert_eq!(icon_offset(1, 2, 31, false), (16, 0));
        assert_eq!(icon_offset(2, 2, 31, false), (0, 19));
        assert_eq!(icon_offset(3, 2, 31, false), (16, 19));
        // From the bottom of a 32-px box, stacking up.
        assert_eq!(icon_offset(0, 2, 31, true), (0, 16));
        assert_eq!(icon_offset(2, 2, 31, true), (0, -3));
    }

    #[test]
    fn numbers_only_in_the_last_ten_seconds() {
        let mut r = rec();
        put16(&mut r, BLOODLUST, 750); // just cast: 15 s, no number
        put16(&mut r, UNHOLY_ARMOR, 480); // 10 s left (9.6 rounds up)
        put16(&mut r, HASTE_SLOW, (-49i16) as u16); // Slow: under a second
        assert_eq!(icon_numbers(&r), vec![(1, 1), (2, 10)]);
        // A timer above its spell's length gets no number but keeps its place.
        put16(&mut r, INVISIBILITY, 65535);
        assert_eq!(icon_numbers(&r), vec![(1, 1), (3, 10)]);
    }

    #[test]
    fn stock_rows_follow_the_game() {
        let show = FLAG_SHOW_HEALTH;
        assert!(stock_rows(100, 32, 0, false, 10, 20, 0).is_empty(), "health bars off");
        // Always, above: health at top - 16, a caster's mana right under it.
        let r = stock_rows(100, 32, show, true, 30, 60, 128);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].y, r[0].palette), (84, PALETTE_YELLOW));
        assert_eq!((r[1].y, r[1].palette), (87, PALETTE_MANA));
        // Below: a non-caster's health moves to the second row.
        let r = stock_rows(100, 32, show | FLAG_BELOW, false, 60, 60, 0);
        assert_eq!((r[0].y, r[0].palette), (144, PALETTE_GREEN));
        // Only when damaged: full health shows nothing, a caster's mana only when not full.
        assert!(stock_rows(100, 32, show | FLAG_ONLY_DAMAGED, false, 60, 60, 0).is_empty());
        assert!(stock_rows(100, 32, show | FLAG_ONLY_DAMAGED, true, 60, 60, 255).is_empty());
        let r = stock_rows(100, 32, show | FLAG_ONLY_DAMAGED, true, 60, 60, 100);
        assert_eq!(r, vec![StockRow { y: 84, fill: 100.0 / 255.0, palette: PALETTE_MANA }]);
        assert_eq!(stock_rows(100, 32, show | FLAG_ONLY_DAMAGED, false, 20, 60, 0)[0].palette, PALETTE_RED);
    }

    #[test]
    fn filled_width_rounds_down_like_the_game() {
        assert_eq!(stock_filled(32, 0.0), 0);
        assert_eq!(stock_filled(32, 0.5), 16);
        assert_eq!(stock_filled(32, 0.99), 31);
        assert_eq!(stock_filled(32, 1.0), 32);
    }
}
