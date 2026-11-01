//! The countdown's sounds as WAV files (pure): the background beeps, copied from
//! w2r-countdown-sound (Beau Hastings), which beeps for the lobby's start countdown the same way
//! (plugins do not share code; see LICENSE); and the game's own tick file scaled to the game's
//! sound effects volume (`scale_wav`, the game's volume rule).

pub const DEFAULT_VOLUME: i32 = 60;
pub const SAMPLE_RATE: u32 = 22_050;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// A second of the countdown ran down.
    Tick,
    /// The countdown reached zero: the game resumes.
    Go,
}

/// Frequency and length of a generated cue (the same as w2r-countdown-sound's).
pub fn tone(cue: Cue) -> (f32, u32) {
    match cue {
        Cue::Tick => (880.0, 120),
        Cue::Go => (1320.0, 450),
    }
}

fn header(out: &mut Vec<u8>, channels: u16, rate: u32, bits: u16, data_len: u32) {
    let align = channels * (bits / 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * align as u32).to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
}

/// A mono 16-bit PCM WAV file of a sine tone with a short fade in and out (no clicks).
/// `volume` is 0..=100 and is clamped.
pub fn wav(freq: f32, ms: u32, volume: i32) -> Vec<u8> {
    let n = (SAMPLE_RATE as u64 * ms as u64 / 1000) as usize;
    let gain = volume.clamp(0, 100) as f32 / 100.0 * 0.6 * i16::MAX as f32;
    let fade = (SAMPLE_RATE as usize / 200).min(n / 2).max(1);
    let mut out = Vec::with_capacity(44 + n * 2);
    header(&mut out, 1, SAMPLE_RATE, 16, (n * 2) as u32);
    for i in 0..n {
        let edge = i.min(n - 1 - i);
        let env = if edge < fade {
            edge as f32 / fade as f32
        } else {
            1.0
        };
        let t = i as f32 / SAMPLE_RATE as f32;
        let v = (t * freq * core::f32::consts::TAU).sin() * env * gain;
        out.extend_from_slice(&(v as i16).to_le_bytes());
    }
    out
}

/// The game's lobby countdown tick, next to the game's exe (a loose 16-bit PCM file in build 2818).
pub const TICK_FILE: &str = r"Data\Sound\Glue\countdown.wav";

/// The entry of the game's volume table a sound effects volume (0 to 100) picks, as the game's
/// UI sound player computes it.
pub fn volume_index(sfx_volume: i32) -> usize {
    (sfx_volume.clamp(0, 100) * 99 / 100) as usize
}

/// The loudness factor of a DirectSound volume (hundredths of a decibel, 0 is full volume).
pub fn gain_of(hundredths_db: i32) -> f32 {
    10f32.powf(hundredths_db.clamp(-10_000, 0) as f32 / 2000.0)
}

/// A PCM WAV file (8 or 16 bit) with every sample scaled by `gain`; `None` for anything else.
pub fn scale_wav(file: &[u8], gain: f32) -> Option<Vec<u8>> {
    if file.len() < 12 || &file[0..4] != b"RIFF" || &file[8..12] != b"WAVE" {
        return None;
    }
    let (mut format, mut data) = (None, None);
    let mut at = 12;
    while at + 8 <= file.len() {
        let id = &file[at..at + 4];
        let len = u32::from_le_bytes(file[at + 4..at + 8].try_into().ok()?) as usize;
        let body = file.get(at + 8..at + 8 + len)?;
        match id {
            b"fmt " if len >= 16 => format = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are padded to an even length.
        at += 8 + len + (len & 1);
    }
    let (f, data) = (format?, data?);
    let u16_at = |i: usize| u16::from_le_bytes([f[i], f[i + 1]]);
    let (tag, channels, bits) = (u16_at(0), u16_at(2), u16_at(14));
    let rate = u32::from_le_bytes(f[4..8].try_into().ok()?);
    if tag != 1 || !(1..=2).contains(&channels) || !matches!(bits, 8 | 16) || rate == 0 {
        return None;
    }
    let gain = gain.clamp(0.0, 1.0);
    let frame = (channels * bits / 8) as usize;
    let data = &data[..data.len() / frame * frame];
    let mut out = Vec::with_capacity(44 + data.len());
    header(&mut out, channels, rate, bits, data.len() as u32);
    if bits == 16 {
        for s in data.chunks_exact(2) {
            let v = i16::from_le_bytes([s[0], s[1]]) as f32 * gain;
            out.extend_from_slice(&(v as i16).to_le_bytes());
        }
    } else {
        // 8-bit PCM is unsigned, centred on 128.
        out.extend(
            data.iter()
                .map(|&u| ((u as f32 - 128.0) * gain + 128.0) as u8),
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_volume_rule() {
        assert_eq!(volume_index(100), 99);
        assert_eq!(volume_index(50), 49);
        assert_eq!(volume_index(0), 0);
        assert_eq!(volume_index(250), 99, "clamped");
        assert!((gain_of(0) - 1.0).abs() < 1e-6);
        assert!((gain_of(-600) - 0.501).abs() < 0.01, "-6 dB is about half");
        assert!(gain_of(-4000) < 0.011);
    }

    #[test]
    fn a_wav_file_is_scaled() {
        let mut file = Vec::new();
        header(&mut file, 1, 22_050, 16, 4);
        file.extend_from_slice(&1000i16.to_le_bytes());
        file.extend_from_slice(&(-1000i16).to_le_bytes());
        let out = scale_wav(&file, 0.5).unwrap();
        assert_eq!(out.len(), file.len());
        assert_eq!(i16::from_le_bytes([out[44], out[45]]), 500);
        assert_eq!(i16::from_le_bytes([out[46], out[47]]), -500);
        assert!(scale_wav(b"not a wav file", 1.0).is_none());
        let mut float = file.clone();
        float[20] = 3; // IEEE float, not PCM
        assert!(scale_wav(&float, 1.0).is_none());
    }

    #[test]
    fn wav_header_and_length_are_consistent() {
        let (f, ms) = tone(Cue::Tick);
        let w = wav(f, ms, 60);
        let n = (SAMPLE_RATE as usize * ms as usize) / 1000;
        assert_eq!(w.len(), 44 + n * 2);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            w.len() - 8
        );
    }

    #[test]
    fn volume_scales_and_zero_is_silent() {
        let peak = |v| {
            wav(880.0, 120, v)[44..]
                .chunks(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
                .max()
                .unwrap()
        };
        assert_eq!(peak(0), 0);
        assert!(peak(100) > peak(50) && peak(50) > 0);
    }
}
