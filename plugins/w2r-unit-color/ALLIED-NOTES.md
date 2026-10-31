# Allied Minimap Colors: research notes

How Allied Minimap Colors was worked out, for whoever has to fix it after a game update.
User-facing documentation is in [README.md](README.md#allied-minimap-colors).

## Credit

Idea and method: **Lambchops**, *Allied Colors* (`allied_color.w2p`,
<https://forum.war2.ru/index.php?topic=4910.15>) for the Warcraft II
Battle.net Edition Combat Edition. No code from it is used here; the `.w2p` is not included in this
repository (it is someone else's binary, and the plugin rules allow source only).

## What the Combat Edition plugin did

`allied_color.w2p` is a 32-bit Windows DLL (`AllCol.dll`, export `w2p_init`, built May 2019) for
the old Battle.net Edition executable. Read from its disassembly:

- A thread polls `GetAsyncKeyState(VK_INSERT)` every 300 ms; each press toggles.
- On: for each of the 8 players, reads the alliance byte at `0x4abd54 + local * 16 + player`
  (local player at `0x4acb18`); allied gets palette entry `0x01` (blue), otherwise `0xd0` (red).
  Writes the 8 bytes to the live minimap color table at `0x4a48cc`.
- Off: copies the stock table (`0x4a48ac`) back through the player-to-color map.
- Its data holds the stock table `d0 01 d8 dc e0 e4 ff 02`, the same bytes found in Remastered.

## Where the table is in Remastered (build 1.0.2.2818)

Found by searching `Warcraft II.exe` for `d0 01 d8 dc e0 e4 ff 02` in `.data`:

| Address | What | Evidence |
|---|---|---|
| `0x008c8d84` | stock minimap dot per **color** index | copied at match start: `0x004f09c3` `mov al, [ecx+0x8c8d84]` with `ecx = TeamColourReorder[player]` |
| `0x008c8d64` | live minimap dot per **player** | written at `0x004f09c9`; read by the dot drawing at `0x004c2d6b` and `0x004f8531` (`mov al, [owner+0x8c8d64]`, then a filled rectangle) |

Neither is a host symbol, so the plugin takes `TeamColourRgba` (`0x008c9640`) minus `0x8dc`, then
refuses to use it unless the stock copy 0x20 bytes later reads exactly `d0 01 d8 dc e0 e4 ff 02`.

Confirmed in game (2026-10-26): the checkbox turns allies blue and enemies red on the minimap.

## Other symbols used

- `LocalPlayer` (u8), `DiplomacyStance` (`[player * 16 + other]`, non-zero = allied),
  `TeamColourReorder` (player to color index), as the host's minimap ping reads them.
- `ChatTargetMode`: non-zero while the chat box is open (w2r-chat reads it the same way). The first
  version used `ChatDialog`, which stays set for the whole match, so every hotkey press was ignored.

## Hotkeys

The host has no keyboard API for plugins; the per-frame tick (`HudMenuButtons` detour) polls
`GetAsyncKeyState`, only while the game window is in front (`GetForegroundWindow` belongs to this
process) and the chat box is closed. A plugin cannot keep a key from the game.

In-game tests (2026-10-26): F2, F3 and F4 jump the camera to a saved location (F4 also switches
between the current fight and the start location); F6, F7 and Insert are used by the game; F1 and
F8 do nothing in the game. Offered: F1 (default), F8, and F2/F3/F4 marked "also moves the camera".

## After a game update

If the log says `Allied Minimap Colors off: minimap color table not where build 1.0.2.2818 has it`,
search the new `Warcraft II.exe` for `d0 01 d8 dc e0 e4 ff 02` again: two copies 0x20 apart, the
lower one is the live per-player table. Update `MINIMAP_BEFORE_TABLE` in `src/plugin.rs` to
`TeamColourRgba - live`, and add the new build to `game_builds` in `plugin.json`.
