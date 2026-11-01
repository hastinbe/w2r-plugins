# Pause Pro

Chooses how the map looks while a match is paused, shows a box saying who paused, and counts down
5 seconds before a resume.

- **Pause Screen** (on the options screen under UI, Pause Pro UI, and as a dropdown on the plugin's
  card):
  - **Default** (the default): the game's own grey-out.
  - **Recolor**: no greyscale; the map stays in colour. The Pause Box is always shown with it (it
    is then the only sign of the pause): its checkbox is stored ticked while Recolor is chosen,
    and stays ticked after switching away from Recolor until you untick it.
  - **Tint**: the grey takes the colour of the player who paused (online: whoever's pause it is;
    offline: you). Dark colours are brightened so they still show.

  A change applies at once, also while the match is paused: on the options screen when you press
  Apply or Save and Exit, on the card when you pick it.
- **Pause Box** (same places, a checkbox; on by default, and always on with Recolor: while Recolor is chosen
  the options screen shows it ticked and greyed out): while a match
  is paused, a box over the map view in your race's stock dialog art (the frame of the game's Tips
  dialog):
  "PAUSED", who paused (their colour swatch, outlined white when it is you, as Player Color Bar
  draws it, and their name) and, online, "Pauses: X used, Y left" (every player has 3 per online
  match). Offline the box says "You" and shows no count: there is no limit, and the game's name
  table still holds the previous online match's names. The box is drawn in the game's dialog
  layer, after the grey-out or tint, so it is never greyed or tinted. It never covers another
  dialog: while a menu, the Tips dialog, Alliances or any other dialog of the match is open, the
  box is not drawn; and it never takes the mouse or keyboard, so chat works while paused.
- **Pause Box Position** (same places, a dropdown; "Center (Default)" by default): the map view's 3 x 3 grid
  (Top, Middle or Bottom; Left, Center or Right) or Over Selection Display (top-aligned on the panel below the
  minimap, the box kept inside the picture). The places follow
  the window size and the UI scale, in the HD view and in the classic view (F5), which has its own
  layout: its HUD is fitted to the window's height with black bars at the sides.
- **Resume Countdown** (same places, a checkbox; on by default): pressing Resume (the HUD button,
  the hotkey or the pause menu) while paused starts a 5-second countdown, as the lobby's start
  countdown, instead of resuming at once. The Pause Box shows "RESUMING IN 5" down to 1; with the
  Pause Box off, the message area shows "Resuming in 5" down to 1 instead (the game's own line for
  its "paused" notice, each second replacing the last). At zero the plugin sends the game's own
  resume command. Pressing Resume again cancels your countdown; a press during someone else's
  countdown does nothing. No pauses are charged.
  - **Sound:** with the game in focus, the lobby countdown's own tick each second (the game's
    `Data/Sound/Glue/countdown.wav`), at your sound effects volume, scaled the way the game scales
    it (silent when sound is off or that volume is 0). With the game in the
    background, where the game is silent, Countdown Sound's beeps: one each second and a higher one
    at zero.
  - **Other players with the plugin** see and hear the countdown too (this needs a host that lets
    plugins message peers during a match). If two of you press Resume at about the same time, the
    lower player number's countdown is kept on every machine, so one resume is sent.
  - **Players without the plugin or the mod loader** are not affected: the resume is the game's own
    command, so every player resumes at the same turn as always, and nothing is sent to them. A
    resume sent twice (if two players' plugins both send one) does nothing the second time on
    every machine alike: the game ignores a resume when the match is not paused. A stock player's
    Resume still resumes at once for everyone and ends any countdown.
  - **Safety:** if the countdown cannot run (the box's frame hook is missing, or the setting is
    off), Resume acts as normal; a countdown whose resume never takes effect is dropped 3 seconds
    after it should have ended.
- **Countdown Volume** (same places, a slider; 60 by default, 0 is silent): the background beeps
  only. In the game the tick plays at your sound effects volume.
- **How:** the game greys the screen with a full-screen shader that its pause command switches on
  (mode 1) and its resume command switches off (mode 0). Recolor passes "off" instead of the grey.
  Tint builds a tinted copy of the game's grey shader with the game's own shader builder, puts it
  into the effect's mode-2 slot (stock: a red-only grey that nothing uses), sets the pauser's colour
  at each pause and asks for mode 2. The program is only built once Tint is chosen. The box follows
  the steps of the game's own victory popup: the race's dialog skin, a window in the dialog layer,
  the contents. If anything is missing, the log says why and the stock behaviour stays.
- **The simulation is unchanged.** The pause, the pause count and the resume command are the game's
  own; the countdown only sends the resume later.
- **Needs a host with the symbols** `SetPostEffect` (all choices); `PostEffect`,
  `PostEffectVertexSource`, `BuildShaderProgram`, `ShaderSetInt`, `ShaderSetVec3` (Tint);
  `VictoryPopupBuild`, `DialogPopupRect`, `DialogSkinManager`, `StockPopupSkinHuman`,
  `StockPopupSkinOrc` (Pause Box); `PauseButtonAction`, `MatchPauseFlags`, `NetworkedMatch`, and optionally `WriteStickyMessage`,
  `SoundEnabled`, `SfxVolume`, `SoundVolumeTable`, `LocalPlayer` and `NetSlotPlayer` (Resume Countdown). An older host logs which one is missing.
  These symbols, plugin messages during a match and the locked checkbox are on the loader branch
  [Maxx444/w2r-mod-loader `feat/pause-pro-host`](https://github.com/Maxx444/w2r-mod-loader/tree/feat/pause-pro-host),
  offered alongside this plugin.
- Build 1.0.2.2818 only.

## Status

- **Phase 1** (this version): Pause Screen, Pause Box and Resume Countdown.
- **Phase 2, in development:** a ready check before a resume (who is ready, with a timer).
- The version keeps the `-mx` suffix of the author's own builds; drop it if this plugin is
  published.

## Known limitations and testing

- Tested in game on a custom scenario match: the three Pause Screen choices, the Pause Box in
  every position in the HD and the classic view, and the countdown's tick, beeps and message line.
- **Not yet tested:** an online match against other players (the countdown shared between
  machines, the tie-break, players without the plugin or the mod loader), observers and replays.
- On a loader without `feat/pause-pro-host` the symbols above are missing: the plugin logs which
  ones and the game's own pause stays unchanged.
- A press of Resume by a player without the plugin resumes at once, as always.

## Credits

The countdown's background beeps are copied from
[Countdown Sound](https://github.com/hastinbe/w2r-plugins/tree/main/plugins/w2r-countdown-sound)
by **Beau Hastings** (hastinbe). See [LICENSE](LICENSE): no license was granted for that code; it is
offered back to Beau with this plugin, and publishing it needs his permission.

## Capabilities

- `game.read`: `symbol_lookup` finds the symbols above.
- `hooks.raw`: `hook_install` on `SetPostEffect` (static 0x5a1ba0) and `VictoryPopupBuild` (0x547d70,
  the box and the countdown's tick); `symbol_address` for Tint and the box, which call the game's own shader and drawing
  functions. Tint also writes one pointer, the effect's mode-2 program slot (put back on unload).
  `PauseButtonAction` (0x4f2740) is hooked for the resume countdown, which calls the game's own
  function at zero.
- `net.send`: `peer_send` tells the other players with the plugin that a countdown started or was
  cancelled (3 bytes at most, once per press).
