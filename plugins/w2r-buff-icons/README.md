# Buff Icons

**Test build (0.1.2-dev).** Tidier spell icons and health bars over units.

- **Packed icons.** The game puts each spell icon at a fixed spot 16 px apart (Bloodlust, Haste or
  Slow, Invisibility, Unholy Armor), so a lone Unholy Armor sits 48 px out from the unit. Here the
  icons sit side by side from the left with no gaps, and wrap to a new row by unit size: 2 per row
  on most units, 3 on Knights, Ogres, Paladins and Ogre Mages, 4 on ships, flyers and siege. Rows
  are an icon plus 3 px apart. Always on.
- **Buff Icons Follow Health Bars** (on by default): with the game's health bars set below units,
  the icons start at the bottom of the unit and stack up. Off: they stay at the top, as the game
  draws them.
- **Countdown on Buff Icons** (on by default): in a spell's last 10 seconds, the seconds left in
  the game's font, see-through white over the icon. Only on your units and the units of players
  whose vision you share; never on enemies.
- **See-Through Health Bars** (on by default): the health and mana bars over units get a
  see-through black instead of solid black. The plugin then draws them itself with the game's
  rules (always or only when damaged, above or below, green / yellow / red by health left, a
  caster's mana under it).

The settings are on **Options > Gameplay**: See-Through Health Bars under the Health Bars heading,
and the two icon settings right after it under **Buff Icons**. The same settings are on the
plugin's card in the plugin manager (Manage, on the title screen), in the same order.

## Logging

Nothing is written to w2r.log. **Debug Log** (on the plugin manager card only, off by default)
writes the plugin's own `debug.log` in its data folder; the two runs before are kept as
`debug.1.log` and `debug.2.log`.

## Needs

Warcraft II: Remastered 1.0.2.2818 and **W2R Mod Loader 1.2.1 or later** (pending: the loader
changes `feat/unit-bar-symbols` and `feat/map-draw`). That is the first core with:

- the symbols `UnitBuffIcons`, `DrawBuffIcon`, `DrawUnitHealthBar`, `UnitSelectBox`,
  `UnitDrawCentre` and `UnitMaxHp` (with the existing `UnitTypeFlags`, `GameSettingsFlags`,
  `ViewPlayer`, `VisionMasks` and `RenderPalette`);
- map drawing for plugins (the `w2r_map_draw` export), for the countdown and the see-through bars.

On an older loader the plugin does nothing and its card shows an error. With the symbols but
without map drawing, the icons are still packed, the game keeps drawing its own health bars and
there is no countdown. The Health Bars heading comes last on the Gameplay page only with the
loader change `feat/health-bars-last`; without it the rows still appear, in the stock order.

## Capabilities

- `game.read`: look up the symbols above, and read the unit type, settings and vision tables.
- `hooks.raw`: three detours, none of which changes game state (`affects_simulation` is false):
  - `UnitBuffIcons` notes which unit's icons are being drawn and calls the original.
  - `DrawBuffIcon` calls the original with the icon's position moved to its packed spot.
  - `DrawUnitHealthBar`: with See-Through Health Bars off it calls the original; on, it skips it
    and the same bars are drawn by the plugin instead. Either way it notes the unit's countdown
    numbers.

## Known limits and untested

- The countdown assumes 50 game ticks a second (the speed it was measured at); at other game
  speeds the seconds run faster or slower.
- The packed icons, the bars, the numbers and both settings screens were checked in the game in a
  test build (HD and Classic graphics, single player and online against a friend, health bars above
  and below). Not yet tested: observers, replays, other screen sizes.
- Hooks the same game functions as any other plugin that moves the spell icons or redraws the
  health bars; only one such plugin should be enabled.

## License

MIT, see `LICENSE` (Copyright (c) 2026 Maxx444).
