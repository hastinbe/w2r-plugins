//! The resume countdown (32-bit only). A hook on `PauseButtonAction`, which every Pause and Resume
//! press runs (the HUD button, the hotkeys, the pause menu): while the match is paused, the press
//! starts a 5-second countdown instead of resuming, and when it reaches zero the plugin calls the
//! game's own function, which sends the stock resume command. Every player resumes at the same
//! turn as always; the simulation is untouched, only the command is sent later.
//!
//! - The countdown shows in the Pause Box ("RESUMING IN 5" ... 1), or as the game's sticky line in
//!   the message area when the box is off ("Resuming in 5").
//! - Sounds as w2r-countdown-sound plays the lobby countdown: with the game in focus, the game
//!   lobby's tick plays (`Data\Sound\Glue\countdown.wav`, scaled by the game's own volume table
//!   to the sound effects volume: the game's sound 0 is that tick only in the lobby); in the
//!   background, where the game is silent, a helper thread plays generated beeps, a higher one at
//!   zero, through `PlaySoundW`.
//! - Players with the plugin are told by a plugin message (in a match: needs a host that opens
//!   `peer_send` in a match), and see and hear the same countdown.
//!
//! Players without the plugin or without the mod loader: the plugin never touches the simulation.
//! The resume is the game's own command, run by every client at the same turn; messages go only to
//! machines whose loader announced itself in the lobby; and a second resume (two players' copies
//! both sending one) is ignored by every client alike, since the game's resume handler does
//! nothing when the match is not paused. A stock player's Resume still resumes at once, for
//! everyone, and ends any countdown showing.
//!
//! - Safety: whenever the countdown cannot run (the setting is off, the frame tick is not running,
//!   the game is not paused), a press acts as normal. Pressing again during your own countdown
//!   cancels it. A countdown whose resume never takes effect is dropped after a short grace.
//!
//! The tick runs from the pause box's per-frame hook (`VictoryPopupBuild`), on the main thread.
//!
//! The generated beeps are copied from w2r-countdown-sound by Beau Hastings (see LICENSE).

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use w2r_abi::*;

use crate::model::{self, Countdown, CountdownStep, Message};
use crate::plugin::{handle_of, host, log, lookup, s, setting};
use crate::tone::{self, Cue};

type VoidFn = unsafe extern "C" fn();
type StickyFn = unsafe extern "C" fn(text: *const c_char, duration_ms: u32);

const SND_NODEFAULT: u32 = 0x0002;
const SND_MEMORY: u32 = 0x0004;
const MAX_PLAYERS: u32 = 8;
const MAX_PATH: usize = 260;

#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const u16, module: *mut c_void, flags: u32) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
}

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static HOOK: Mutex<Option<W2rHandle>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(true);
static VOLUME: AtomicI32 = AtomicI32::new(tone::DEFAULT_VOLUME);
static COUNTDOWN: Mutex<Option<Running>> = Mutex::new(None);
/// When the frame tick last ran (ms since `EPOCH`, 0 for never).
static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();
static PAUSE_FLAGS: AtomicUsize = AtomicUsize::new(0);
static NETWORKED: AtomicUsize = AtomicUsize::new(0);
static LOCAL_PLAYER: AtomicUsize = AtomicUsize::new(0);
static NET_SLOT_PLAYER: AtomicUsize = AtomicUsize::new(0);
/// `WriteStickyMessage` and the game's sound settings (`SoundEnabled`, `SfxVolume`,
/// `SoundVolumeTable`), 0 when the host does not have them.
static STICKY: AtomicUsize = AtomicUsize::new(0);
static SOUND_ENABLED: AtomicUsize = AtomicUsize::new(0);
static SFX_VOLUME: AtomicUsize = AtomicUsize::new(0);
static VOLUME_TABLE: AtomicUsize = AtomicUsize::new(0);
/// The game's lobby countdown tick (`tone::TICK_FILE`), read once at load; `None` when missing.
static TICK: OnceLock<Option<Vec<u8>>> = OnceLock::new();
/// The helper thread, and where to send it WAV files to play.
type SoundThread = (Sender<Vec<u8>>, JoinHandle<()>);
static SOUNDS: Mutex<Option<SoundThread>> = Mutex::new(None);

/// The countdown showing, and the net slot that started it (`None`: the local player).
#[derive(Clone, Copy)]
struct Running {
    countdown: Countdown,
    from: Option<u32>,
}

fn now_ms() -> u64 {
    // +1 so a reading is never 0 (which means "never" for `LAST_FRAME_MS`).
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

fn byte_at(addr: &AtomicUsize) -> Option<u8> {
    let at = addr.load(Ordering::Relaxed);
    // SAFETY: a byte global of the game image, from the host's symbol table.
    (at != 0).then(|| unsafe { core::ptr::read_volatile(at as *const u8) })
}

fn paused() -> bool {
    byte_at(&PAUSE_FLAGS).is_some_and(|f| f & 1 != 0)
}

fn networked() -> bool {
    byte_at(&NETWORKED).is_some_and(|n| n != 0)
}

// ---- sounds ------------------------------------------------------------------------------

/// The helper thread: plays each WAV to the end (`PlaySoundW` is synchronous, so the WAV outlives
/// the playback), until the plugin unloads and drops the sender.
fn sounds(wavs: mpsc::Receiver<Vec<u8>>) {
    for wav in wavs {
        unsafe {
            PlaySoundW(
                wav.as_ptr().cast(),
                core::ptr::null_mut(),
                SND_MEMORY | SND_NODEFAULT,
            )
        };
    }
}

/// True when the foreground window belongs to this process (the game has focus). False when it is
/// in the background, minimised, or nothing has focus; the game plays its sounds only while
/// focused. (As w2r-countdown-sound decides it.)
fn game_has_focus() -> bool {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, &mut pid);
        pid == GetCurrentProcessId()
    }
}

/// The game's own lobby tick file as it would sound now: scaled to the game's sound effects
/// volume by the game's own volume table, as the game plays it in the lobby. `Some(None)` when the
/// game's sound is off or that volume is 0 (silent, as the game would be); `None` when the file
/// or a setting is missing.
fn game_tick() -> Option<Option<Vec<u8>>> {
    let file = TICK.get()?.as_ref()?;
    let (enabled, volume, table) = (
        SOUND_ENABLED.load(Ordering::Relaxed),
        SFX_VOLUME.load(Ordering::Relaxed),
        VOLUME_TABLE.load(Ordering::Relaxed),
    );
    if enabled == 0 || volume == 0 || table == 0 {
        return None;
    }
    // SAFETY: the game's sound globals, from the host's symbol table (checked by the guard
    // signature of the game's UI sound player, which reads them the same way).
    let (enabled, volume) = unsafe {
        (
            core::ptr::read_volatile(enabled as *const u32),
            core::ptr::read_volatile(volume as *const i32),
        )
    };
    if enabled == 0 || volume <= 0 {
        return Some(None);
    }
    let db =
        unsafe { core::ptr::read_volatile((table as *const i32).add(tone::volume_index(volume))) };
    Some(tone::scale_wav(file, tone::gain_of(db)))
}

/// A cue: in focus the game's own lobby tick at the game's sound effects volume (the lobby has no
/// sound at zero: the start is the cue); in the background, where the game is silent,
/// w2r-countdown-sound's beeps at the Countdown Volume, a higher one at zero. Without the tick
/// file, the beeps play in focus too.
fn play(cue: Cue) {
    let tick = if game_has_focus() { game_tick() } else { None };
    let wav = match (tick, cue) {
        (Some(tick), Cue::Tick) => tick,
        (Some(_), Cue::Go) => None,
        (None, _) => {
            let volume = VOLUME.load(Ordering::Relaxed);
            (volume > 0).then(|| {
                let (freq, ms) = tone::tone(cue);
                tone::wav(freq, ms, volume)
            })
        }
    };
    if let Some(wav) = wav
        && let Some((tx, _)) = SOUNDS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
    {
        let _ = tx.send(wav);
    }
}

/// Reads the game's tick file from the game's folder (next to its exe).
fn read_tick() -> Option<Vec<u8>> {
    let mut buf = [0u16; MAX_PATH];
    let n =
        unsafe { GetModuleFileNameW(core::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    let exe = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]));
    let file = std::fs::read(exe.parent()?.join(tone::TICK_FILE)).ok()?;
    // Only a file the plugin can scale.
    tone::scale_wav(&file, 1.0).map(|_| file)
}

/// The countdown in the message area (the game's sticky line, as its "paused" notice), when the
/// Pause Box is off. Each line replaces the last.
fn write_line(n: u32) {
    let sticky = STICKY.load(Ordering::Relaxed);
    if sticky == 0 || crate::pause_box::shown() {
        return;
    }
    let Ok(text) = std::ffi::CString::new(model::countdown_line(n)) else {
        return;
    };
    let sticky: StickyFn = unsafe { core::mem::transmute(sticky) };
    unsafe { sticky(text.as_ptr(), model::LINE_MS) };
}

/// The player at net slot `n`, when it is one.
fn player_of(n: u32) -> Option<u32> {
    let table = NET_SLOT_PLAYER.load(Ordering::Relaxed);
    if table == 0 || n >= MAX_PLAYERS {
        return None;
    }
    // SAFETY: the game's net-slot-to-player table, 8 u32s, from the host's symbol table.
    let p = unsafe { core::ptr::read_volatile((table as *const u32).add(n as usize)) };
    (p < MAX_PLAYERS).then_some(p)
}

// ---- messages ----------------------------------------------------------------------------

/// Tells the other players with the plugin; a failure (offline, an older host) is fine: they just
/// do not see the countdown, and the resume still reaches them.
fn send(m: Message) {
    if !networked() {
        return;
    }
    let Some(h) = host() else { return };
    let bytes = model::encode(m);
    let buf = W2rBuf {
        ptr: bytes.as_ptr(),
        len: bytes.len() as u32,
    };
    let status = unsafe { (h.peer_send)(h.token, W2R_SLOT_ALL, buf) };
    if status != W2R_OK {
        static WARNED: AtomicU32 = AtomicU32::new(0);
        if WARNED.swap(1, Ordering::Relaxed) == 0 {
            log(
                W2R_LOG_INFO,
                &format!(
                    "resume countdown: other players not told (peer_send status {status}); the resume still waits"
                ),
            );
        }
    }
}

/// A plugin message from another player in the match.
pub fn on_peer(m: &W2rEvtPeerMessage) {
    if m.data.ptr.is_null() {
        return;
    }
    let bytes = unsafe { core::slice::from_raw_parts(m.data.ptr, m.data.len as usize) };
    let Some(message) = model::decode(bytes) else {
        return;
    };
    let now = now_ms();
    let mut current = COUNTDOWN.lock().unwrap_or_else(|e| e.into_inner());
    match message {
        Message::Start(seconds) if paused() => {
            if let Some(r) = *current
                && r.from.is_none()
            {
                // Mine is running too (both pressed at about the same time) or has already sent
                // its resume: keep mine unless theirs wins the tie, so one resume is sent.
                let local = byte_at(&LOCAL_PLAYER).unwrap_or(u8::MAX);
                let theirs = player_of(m.net_slot).unwrap_or(u32::MAX);
                if r.countdown.remaining(now) == 0 || !model::theirs_wins(theirs, local) {
                    log(
                        W2R_LOG_INFO,
                        &format!(
                            "resume countdown: net slot {} started one too; keeping mine",
                            m.net_slot
                        ),
                    );
                    return;
                }
            }
            *current = Some(Running {
                countdown: Countdown::new(now, seconds, false),
                from: Some(m.net_slot),
            });
            drop(current);
            log(
                W2R_LOG_INFO,
                &format!("resume countdown: started by net slot {}", m.net_slot),
            );
        }
        Message::Cancel if current.is_some_and(|r| r.from == Some(m.net_slot)) => {
            *current = None;
            log(
                W2R_LOG_INFO,
                &format!("resume countdown: cancelled by net slot {}", m.net_slot),
            );
        }
        _ => {}
    }
}

// ---- the press and the tick --------------------------------------------------------------

unsafe fn call_original() {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        let original: VoidFn = unsafe { core::mem::transmute(original) };
        unsafe { original() };
    }
}

/// What a press does: true when the plugin took it (started or cancelled a countdown), false to
/// let the game act as normal.
fn press() -> bool {
    if !ENABLED.load(Ordering::Relaxed) || !paused() {
        return false;
    }
    let now = now_ms();
    let fresh = now.saturating_sub(LAST_FRAME_MS.load(Ordering::Relaxed)) <= model::FRAME_FRESH_MS;
    let mut current = COUNTDOWN.lock().unwrap_or_else(|e| e.into_inner());
    let running = current.map(|r| (r.from.is_none(), r.countdown.remaining(now) > 0));
    match running {
        // Pressing again during your own countdown cancels it.
        Some((true, true)) => {
            *current = None;
            drop(current);
            send(Message::Cancel);
            log(W2R_LOG_INFO, "resume countdown: cancelled");
            true
        }
        // Someone else's countdown is running: it resumes the game for everyone soon.
        Some((false, true)) => true,
        // Nothing would finish a countdown: resume now.
        _ if !fresh => false,
        // A finished countdown whose resume has not shown yet, or none: start one.
        _ => {
            *current = Some(Running {
                countdown: Countdown::new(now, model::COUNTDOWN_SECONDS, true),
                from: None,
            });
            drop(current);
            send(Message::Start(model::COUNTDOWN_SECONDS));
            log(W2R_LOG_INFO, "resume countdown: started");
            true
        }
    }
}

unsafe extern "C" fn pause_button_detour() {
    let taken = std::panic::catch_unwind(press).unwrap_or(false);
    if !taken {
        unsafe { call_original() };
    }
}

/// One match frame (main thread, from the pause box's hook): advances the countdown, plays its
/// sounds and, at zero, sends the resume of a countdown the local player started.
pub fn tick() {
    let now = now_ms();
    LAST_FRAME_MS.store(now, Ordering::Relaxed);
    let mut current = COUNTDOWN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(Running { countdown: c, .. }) = current.as_mut() else {
        return;
    };
    if !paused() {
        // Resumed (by the countdown or by anyone): done.
        *current = None;
        return;
    }
    if c.stalled(now) {
        *current = None;
        log(
            W2R_LOG_INFO,
            "resume countdown: the resume did not take effect; dropped",
        );
        return;
    }
    let step = c.step(now);
    let mine = c.mine;
    drop(current);
    match step {
        CountdownStep::Tick(n) => {
            play(Cue::Tick);
            write_line(n);
        }
        CountdownStep::Go => {
            play(Cue::Go);
            write_line(0);
            if mine {
                // The game's own Resume: it sends the stock resume command (offline it queues it).
                unsafe { call_original() };
            }
        }
        CountdownStep::None => {}
    }
}

/// The title for the box while a countdown runs ("RESUMING IN 3").
pub fn title() -> Option<String> {
    let r = (*COUNTDOWN.lock().unwrap_or_else(|e| e.into_inner()))?;
    Some(model::countdown_title(r.countdown.remaining(now_ms())))
}

// ---- settings ----------------------------------------------------------------------------

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    // Turned off mid-countdown: it runs out as started (the others were told).
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn volume() -> i32 {
    VOLUME.load(Ordering::Relaxed)
}

pub fn set_volume(v: i32) {
    VOLUME.store(v.clamp(0, 100), Ordering::Relaxed);
}

pub fn load_setting() {
    set_enabled(setting(model::COUNTDOWN_KEY, 1) != 0);
    set_volume(setting(model::VOLUME_KEY, tone::DEFAULT_VOLUME as i64) as i32);
}

/// "Resume Countdown" and "Countdown Volume" for the options screen, under Pause Pro UI.
pub fn declare(h: &W2rHost) -> bool {
    if h.struct_size < W2R_HOST_SETTINGS_DECLARE_SIZE {
        return false;
    }
    let def = W2rSettingDef {
        struct_size: W2R_SETTING_DEF_V1_SIZE,
        kind: W2R_SETTING_BOOL,
        default_int: 1,
        min: 0,
        max: 0,
        flags: 0,
        choice_count: 0,
        choices: core::ptr::null(),
        reserved: 0,
        key: s(model::COUNTDOWN_KEY),
        label: s(model::COUNTDOWN_LABEL),
        description: s(model::COUNTDOWN_DESCRIPTION),
        category: s(model::CATEGORY),
        subcategory: s(model::SUBCATEGORY),
        default_str: s(""),
        active_when: s(""),
    };
    let status = unsafe { (h.settings_declare)(h.token, &def) };
    if status != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("declaring {}: status {status}", model::COUNTDOWN_KEY),
        );
    }
    let volume = W2rSettingDef {
        kind: W2R_SETTING_INT,
        default_int: tone::DEFAULT_VOLUME as i64,
        min: 0,
        max: 100,
        key: s(model::VOLUME_KEY),
        label: s(model::VOLUME_LABEL),
        description: s(model::VOLUME_DESCRIPTION),
        active_when: s(model::COUNTDOWN_KEY),
        ..def
    };
    let loud = unsafe { (h.settings_declare)(h.token, &volume) };
    if loud != W2R_OK {
        log(
            W2R_LOG_WARN,
            &format!("declaring {}: status {loud}", model::VOLUME_KEY),
        );
    }
    status == W2R_OK && loud == W2R_OK
}

// ---- install -----------------------------------------------------------------------------

/// Hooks `PauseButtonAction` and starts the sound thread; a reason when the countdown is not
/// available (a host without the symbol). The box's frame hook must be in place for it to run.
pub fn install(h: &W2rHost) -> Result<(), String> {
    let flags = lookup("MatchPauseFlags").ok_or("symbol MatchPauseFlags not available")?;
    let networked = lookup("NetworkedMatch").ok_or("symbol NetworkedMatch not available")?;
    let target = handle_of("PauseButtonAction")
        .ok_or("symbol PauseButtonAction not available (host too old?)")?;
    PAUSE_FLAGS.store(flags, Ordering::Relaxed);
    NETWORKED.store(networked, Ordering::Relaxed);
    // Optional, each with a fallback: without the player tables a tie keeps your own countdown;
    // without the game's sound settings or its tick file the beeps play in focus too; without the
    // sticky line the countdown shows only in the box.
    LOCAL_PLAYER.store(lookup("LocalPlayer").unwrap_or(0), Ordering::Relaxed);
    NET_SLOT_PLAYER.store(lookup("NetSlotPlayer").unwrap_or(0), Ordering::Relaxed);
    STICKY.store(lookup("WriteStickyMessage").unwrap_or(0), Ordering::Relaxed);
    SOUND_ENABLED.store(lookup("SoundEnabled").unwrap_or(0), Ordering::Relaxed);
    SFX_VOLUME.store(lookup("SfxVolume").unwrap_or(0), Ordering::Relaxed);
    VOLUME_TABLE.store(lookup("SoundVolumeTable").unwrap_or(0), Ordering::Relaxed);
    let tick = TICK.get_or_init(read_tick).is_some();
    let settings = [&SOUND_ENABLED, &SFX_VOLUME, &VOLUME_TABLE]
        .iter()
        .all(|a| a.load(Ordering::Relaxed) != 0);
    if !tick || !settings {
        log(
            W2R_LOG_WARN,
            &format!(
                "resume countdown: {}; beeps in game too",
                if tick {
                    "the game's sound settings are not available (host too old?)"
                } else {
                    "the game's countdown tick file was not found"
                }
            ),
        );
    }
    let (tx, rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("w2r-pause-pro-sound".into())
        .spawn(move || sounds(rx))
        .map_err(|e| format!("sound thread: {e}"))?;
    *SOUNDS.lock().unwrap_or_else(|e| e.into_inner()) = Some((tx, worker));
    let mut trampoline: *mut c_void = core::ptr::null_mut();
    let mut hook = 0;
    let status = unsafe {
        (h.hook_install)(
            h.token,
            target,
            pause_button_detour as *mut c_void,
            &mut trampoline,
            &mut hook,
        )
    };
    if status != W2R_OK {
        stop_sounds();
        return Err(format!("PauseButtonAction: hook_install returned {status}"));
    }
    ORIGINAL.store(trampoline as usize, Ordering::Release);
    *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    Ok(())
}

fn stop_sounds() {
    if let Some((tx, worker)) = SOUNDS.lock().unwrap_or_else(|e| e.into_inner()).take() {
        drop(tx);
        // A cue still playing ends within half a second.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

pub fn remove() {
    let hook = HOOK.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let (Some(h), Some(hook)) = (host(), hook) {
        unsafe { (h.hook_remove)(h.token, hook) };
    }
    *COUNTDOWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    stop_sounds();
}
