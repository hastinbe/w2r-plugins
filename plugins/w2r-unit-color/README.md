# Unit Color

Draws any of the eight team colors in a color you choose, on your screen only. Purely cosmetic and
local: nobody else sees it and the simulation is untouched. Grew out of the "Black2Pink" option of
[wc2r-plus](https://github.com/Luxray997/wc2r-plus) by Luxray997 (MIT), by way of the Black Recolor
plugin, which this one replaces.

**Replaces Black Recolor** (`replaces` in `plugin.json`): when Unit Color loads, the loader skips
Black Recolor (`E_REPLACED`), and the manager turns Black Recolor off when Unit Color is installed
enabled. If Unit Color fails to load, Black Recolor keeps running. Needs a W2R Mod Loader that
knows `replaces` (an older one refuses this manifest as `unknown_field`).

**Coming from Black Recolor?** Its pink or cyan setting is not read. Pick the "Black to pink" or
"Black to cyan" profile in Options > Unit Color (or on the plugin card) and press Apply.

## Credits

Based on Black Recolor by Beau Hastings, a port of the Black2Pink option of wc2r-plus by Luxray997
(MIT; see [LICENSE](LICENSE)).

- **Beau Hastings** (hastinbe):
  [Black Recolor](https://github.com/hastinbe/w2r-plugins/tree/main/plugins/w2r-black-recolor),
  which this plugin replaces, and the W2R Mod Loader.
- **Luxray997**: [wc2r-plus](https://github.com/Luxray997/wc2r-plus) (MIT), whose "Black2Pink"
  option Black Recolor ported. Unit Color still carries that code: the ramp shading, the pink and
  cyan values, and how the palette and HD table are written and put back.
- **mauritsschelvis87**: [War_2_Remastered_Mods](https://github.com/mauritsschelvis87-crypto/War_2_Remastered_Mods),
  the inspiration for Unit Color.
- **Lambchops**: *Allied Colors* for the Battle.net Edition
  ([forum.war2.ru](https://forum.war2.ru/index.php?topic=4910.15)), the inspiration for Allied
  Minimap Colors.

## Settings

**Options > Unit Color** (needs a W2R Mod Loader with the options screen):

- **Profiles:**
  - **Color Profile:** a dropdown with **Custom** and **Game default**, then the profiles in
    `color_profiles.json` by name, as they were when the game started. Choose one and press Apply.
    It shows the profile whose colors are in use, or Custom once you change a color by hand.
    There is one **Custom**: it also stands for the saved "Custom" profile (see Save below), and
    choosing it applies that profile's colors when there is one.
    **It lists up to 14 profiles** (the host allows 16 entries, and Custom and Game default take
    two). The six profiles the plugin starts with count toward the 14. Profiles after the 14th
    stay in the file and can be applied from the plugin card; the file holds up to 64.
  - **Save or Delete Profile Name:** the profile the boxes below save or delete. It follows the
    Color Profile dropdown: a profile's name, empty for Game default, and "Custom" when the colors
    are no profile (a name typed but not saved goes back to "Custom" on Apply).
  - **Save Colors as Profile on Apply:** check it and press Apply: the colors below are saved under
    that name, replacing a profile of the same name. Editing a profile is: choose it, Apply, change
    colors, check Save, Apply. Saving as **Custom**, **Game default**, **Classic** or
    **Remastered** saves to the one "Custom" profile (each save replaces it), so the built-in
    profiles are never changed.
  - **Delete This Profile on Apply** and **Yes, Delete It (Cannot Be Undone):** check both and press
    Apply to remove the profile named above. Delete alone does nothing (the "Yes" box is greyed
    until it is checked), and Save with Delete does nothing either. The colors in use are not
    changed.
  - The boxes clear themselves after Apply. A saved or deleted profile joins or leaves the dropdown
    at the next start (the card's list changes at once).
- **Player Colors:** one color picker per team color, `Player 1 (Red)` .. `Player 8 (Yellow)`. Each
  shows a swatch and its hex value; the button opens the color editor with the new and the current
  color side by side. Picking the game's own color means no change for that team.

After Apply the screen shows what the plugin changed (a chosen profile's colors in the pickers, its
name, the cleared boxes). On a W2R Mod Loader that does not refresh the screen after Apply yet,
those show the next time you open Options, and only one profile can be deleted per visit. If you
change the dropdown and a picker in the same visit, the profile wins.

**The plugin's card** (title screen > Manage), for what the options screen cannot do:

| Row | What it does |
|---|---|
| `>> IN USE: ... <<` | Only on a W2R Mod Loader without card dropdowns (otherwise the Color Profile dropdown shows it): the profile whose colors are in use, "Game default", or "Custom colors". |
| `Player 1 (Red)` .. `Player 8 (Yellow)` | One row per team color: on a W2R Mod Loader with card color rows (`W2rUi.color_edit`, hastinbe/w2r-mod-loader#5), a swatch and a `#rrggbb` button that opens the same color editor as Options; otherwise a text box with `#RRGGBB` (type a new one to apply it at once; clear the box for the game's own color). Picking the game's own color leaves that team unchanged. The Options pickers show a change the next time the screen opens. |
| `Color Profile` (dropdown) | On a W2R Mod Loader with card dropdowns (`W2rUi.combo`, hastinbe/w2r-mod-loader#5): the same list as Options > Unit Color (Custom, Game default, then the first 14 profiles in `color_profiles.json`), read live from the file. A pick applies at once: the colors, the swatches and the Profile name box follow. It shows the profile in use, or Custom. Profiles after the 14th cannot be reached from the card. |
| `Profile` + `Apply profile` (only without card dropdowns) | Game default, then every profile in `color_profiles.json`, including ones added since the game started. |
| `Profile name`, `Save current colors as this profile` | Saves the colors in use into `color_profiles.json`, replacing a profile of the same name. Moving the slider fills in that profile's name. |
| `Delete this profile` | Asks "Delete "name"? This cannot be undone." for the profile shown in the Color Profile dropdown (without card dropdowns: chosen with the slider), then `Yes, delete it` removes it from `color_profiles.json` and `No, keep it` does nothing. The colors in use are not changed. Game default, Classic and Remastered cannot be deleted. |
| `Reload color_profiles.json` | Reads the file again after you edit it; the card's list updates at once, the Options dropdown at the next start. |
| `Reset all to game default` | Clears every color. |

On an older W2R Mod Loader without the options screen, the card's text boxes are the only way to
set a color.

The colors are kept as `color1` .. `color8` (`rrggbb`) and the dropdown as `profile` in the
plugin's settings.

## Allied Minimap Colors

> It writes the game's minimap dot table, which has no host symbol. The plugin finds it at a fixed
> distance from `TeamColourRgba` for game build 1.0.2.2818 and uses it only after checking the
> stock bytes next to it (see "How it works"). On any other build it stays off.

Shows **allies in blue and enemies in red on the minimap**, so you can tell sides apart at a
glance whatever colors the match handed out. Your own units keep their color. The units on the
main map are not changed. It is **off by default**.

**Credit:** the idea and the method come from **Lambchops**' *Allied Colors* plugin
(`allied_color.w2p`, [forum.war2.ru](https://forum.war2.ru/index.php?topic=4910.15)) for the
Warcraft II Battle.net Edition Combat Edition. This is a new
implementation for Remastered; none of that plugin's code is used (it was written for the old
executable and cannot load in Remastered).

### Turning it on and off

- **Hotkey: F1** (by default) turns it on or off during a match. It only works while the game
  window is in front and the chat box is closed, so typing it in chat or in another program does
  nothing. Hold-to-repeat does not flicker it: one press, one toggle. A press that is ignored says
  why in the log.
- **Options > Unit Color > Allied Minimap Colors:** the **Allied Minimap Colors** checkbox, and
  **Allied Minimap Colors Hotkey**, a dropdown with F1, F8, F2, F3 and F4.
- **The plugin's card** (title screen > Manage): the same checkbox, and a note with the current
  hotkey.

The hotkey, the checkbox and the options screen are one setting (`allied_minimap`, with the key in
`allied_key`): pressing the hotkey in a match also changes the checkbox, and it stays as you left it for
the next match and the next start.

**F2, F3 and F4 also move the camera.** In Remastered they jump the view to a saved location (your
start location by default; F4 also switches between the current fight and the start location). A
plugin cannot take a key away from the game, so with one of them as the hotkey, every press toggles
the colors *and* moves the camera. They are in the list, marked "(also moves the camera)", for
those who want them anyway. **F6, F7 and Insert are not offered** because the game uses them. The
Combat Edition plugin used Insert; F1 is the default here; in testing the game did nothing with it (nor with F8).

### What you see

| Minimap dots of | With it on |
|---|---|
| You | Your own color (unchanged; a custom Blue or White as described under What changes) |
| Allied players | The stock blue dot |
| Everyone else (enemies) | The stock red dot |

- The red is the first entry of the red team ramp, so a custom **Player 1 (Red)** color also
  changes the enemy dots. The blue is a shared palette entry and is always the stock blue, so a
  custom **Player 2 (Blue)** color does not change the ally dots.
- "Allied" is your side of the game's diplomacy: a player you have set to allied. Change
  alliances in the match and the minimap follows on the next frame.
- In a replay (the W2R Mod Loader's replay viewer) it works from the seat the replay was recorded
  in: that player keeps their color and everyone not allied to them shows red.
- As an observer: not tested. If the game gives no local player among the eight, it does nothing;
  if it does, everyone not allied to that slot shows red. Turn it off there.

### How it works

Remastered still has the Battle.net Edition's eight-byte table of minimap dot colors, one per
player, which the game fills from the players' colors when a match starts. With the feature on,
every frame (the same tick as the color recolor) the plugin writes ally blue or enemy red into it
from the alliance table (`DiplomacyStance`, row `LocalPlayer`), leaving your own entry alone.
Turning it off, or unloading the plugin, writes each player's stock dot back.

The table is not a host symbol. For game build 1.0.2.2818 it sits `0x8dc` bytes before
`TeamColourRgba` (live table 0x008c8d64, stock copy 0x008c8d84, found in the game's minimap
drawing code). At load the plugin checks that the stock copy holds exactly the stock dots
(`d0 01 d8 dc e0 e4 ff 02`); if not, for example after a game update, the feature turns itself off,
writes nothing, logs `Allied Minimap Colors off: ...`, and the card says it is not available. The
rest of Unit Color still works.

## `color_profiles.json`

In `plugins\w2r-unit-color\data\` inside your w2r folder
(`%USERPROFILE%\Saved Games\Warcraft2Remastered\w2r\`). That is the only folder the plugin API lets a
plugin read or write; it never sees a path. Created the first time the plugin loads, with six profiles
(Classic, Remastered, Black to pink, Black to cyan, Maxx444's + Pink, Maxx444's + Cyan).

Each profile is a warcraft2.site / W2 Toolkit preset with a name:

```json
{
  "profiles": [
    {
      "name": "Classic Colors + Black 2 Pink",
      "colors": {
        "players": [
          { "player": 1, "color": "#A40000" },
          { "player": 2, "color": "#003CC0" },
          { "player": 6, "color": "#FF00FF" }
        ],
        "selectionHighlight": "#00FF00",
        "manaBar": "#3470A4"
      }
    }
  ]
}
```

- `player` is the team color's number (1 Red, 2 Blue, 3 Teal, 4 Violet, 5 Orange, 6 Black,
  7 White, 8 Yellow) and `color` is `#RRGGBB`. A player left out keeps the game's own color.
- **Game default**, **Classic** and **Remastered** are protected: saving under one of those names (any case) is refused, so save changes under a different name, and neither the card nor the options screen deletes them. Edit the file by hand to change them.
- The name is printable ASCII, at most 40 characters. An `author` in a pasted site preset is ignored.
- Other fields in `colors` (selection, ally, critter, gold mine and oil highlights, HP and mana
  bars) are not applied, but are kept when the plugin saves the file again.
- A preset copied from warcraft2.site or W2 Toolkit can be pasted in as it is: the file may also be
  a plain list of profiles, the site's listing (`{"items": [...]}`), one profile, or just a
  `colors` object (named "Profile 1").
- A profile with a bad color, a player number outside 1 to 8, or a name used earlier (ignoring case)
  is skipped; the rest still load, and the card and the log say which profile (counting from 1).
  At most 64 profiles and 1 MiB are read.
- If the file is not valid JSON at all, no profiles load and saving is refused, so nothing is
  written over it; fix it and press Reload. The JSON is read strictly (the same reader as the
  W2R Mod Loader's `plugin.json`): a key twice in one object (two `"name"`s in one profile, say,
  from pasting a preset into another), an escaped half of an emoji (`\ud83d` on its own), or more
  than 32 nested lists and objects makes the whole file invalid. A byte order mark at the start
  (Notepad adds one) is fine. `player` must be a whole number written without a decimal point
  (`1`, not `1.0`).
- Saving rewrites the whole file in this layout (two-space indents).

## What changes, and what does not

"Player N" means the Nth team color, as on warcraft2.site; the game shuffles which player gets which
color at the start of a match.

- **HD color table** (`TeamColourRgba`), all eight entries: units, color boxes, and anything else
  that reads the table (the player color bar and chat names in `w2r-chat` follow it).
- **Render palette ramps** for Red to White (entries `0xd0` to `0xeb`, four per team, shaded by the
  stock ramp's brightness steps): classic-graphics sprites, and the minimap dots of the colors whose
  dot uses the ramp's first entry (Red, Teal, Violet, Orange, Black).
- **Minimap dots of Blue and White:** the game draws them with shared palette
  entries (1 and 255) that other things use too, so those entries stay stock. Instead, when Blue or
  White has a custom color, those players' entries in the minimap dot table (the one Allied
  Minimap Colors uses) point at the first entry of their own ramp (`0xd4` and `0xe8`), which
  carries the custom color. Back to the game's own color, the stock entry comes back.
- **Not changed:** Yellow has no ramp of its own (`0xec`.. holds the gold mine highlight) and its
  minimap dot uses the shared entry 2, so Yellow stays stock everywhere except the HD table. The
  War_2_Remastered_Mods color tool has the same limit.
- **Score screen:** the colored bars after a match come from the game's skin file
  (`fe_endgame_stats_bar_0` .. `_7` in `Data\skins\skins.json`), not from the color table. A W2R Mod
  Loader with score screen bars (hastinbe/w2r-mod-loader#7) recolors them
  from the color table, so they follow the colors in use: the bright half is the team's color, the
  dark half is half of it, as in the stock skins. On a loader without it they stay stock.
- **Restores:** choosing Game default, or unloading the plugin, writes the stock values back. The
  settings are kept.

## Capabilities

- `game.read`: symbol lookup for `TeamColourRgba`, `RenderPalette` and `HudMenuButtons`; for
  Allied Minimap Colors also `LocalPlayer`, `DiplomacyStance`, `TeamColourReorder` and `ChatTargetMode`.
- `hooks.raw`: one detour on `HudMenuButtons` (drawn every frame in a match; original called first)
  as the per-frame tick, and direct writes to the two color tables and, with Allied Minimap Colors
  on or a custom Blue or White, the minimap color table (found by offset, see above).
  `affects_simulation` is false: only what your screen draws changes.

## Testing

Tested in game on build 1.0.2.2818:

- multiplayer matches against computer players;
- a **player vs player** match: three people on three computers (one without the mod loader, one
  on an older loader), 27,165 turns on Garden of War, with profiles applied during the match about
  a dozen times. No desync: the loader's sync check found no mismatch. Each player sees only their
  own colors; nothing is sent to other players and the simulation is not touched;
- **Apply in the in-match options** (the pause menu's Options > Unit Color) on all four terrains
  (Forest, Winter, Wasteland, Swamp), in classic and HD graphics, including after switching
  graphics with F5 during the match;
- Allied Minimap Colors and profile changes in a **replay** (the W2R Mod Loader's replay viewer),
  in classic and HD graphics.

Not tested:

- Allied Minimap Colors as an observer;
- the card widgets, the profile dropdown on the card and the score screen bars on a W2R Mod
  Loader release (tested on loaders built from its main branch, which has them since
  hastinbe/w2r-mod-loader#4 to #7 were merged);
- the card's fallback on a loader without those changes (text boxes, slider and `Apply profile`)
  since the card widgets were added in 0.2.0.
