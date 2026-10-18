//! RusTone's sounds, synthesised so they carry no licence. Written once into the
//! cache folder as 16 kHz mono PCM WAV for pjsua's file player.

use std::f32::consts::TAU;
use std::io;
use std::path::{Path, PathBuf};

const RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    /// Incoming call (loops).
    Ringtone,
    /// Local ringback while the far end rings without early media (loops).
    Ringback,
    /// A second call arriving during a call.
    CallWaiting,
    /// The call ended.
    Hangup,
    MessageIn,
    MessageOut,
}

impl Sound {
    pub const ALL: [Sound; 6] = [Self::Ringtone, Self::Ringback, Self::CallWaiting, Self::Hangup, Self::MessageIn, Self::MessageOut];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Ringtone => "ringtone.wav",
            Self::Ringback => "ringback.wav",
            Self::CallWaiting => "call-waiting.wav",
            Self::Hangup => "hangup.wav",
            Self::MessageIn => "message-in.wav",
            Self::MessageOut => "message-out.wav",
        }
    }

    /// Mono samples in -1..=1 at 16 kHz.
    pub fn samples(self) -> Vec<f32> {
        match self {
            Self::Ringtone => ringtone(),
            Self::Ringback => ringback(),
            Self::CallWaiting => cadence(&[(0.2, Some(425.0)), (0.2, None), (0.2, Some(425.0))], 0.35),
            Self::Hangup => [note(660.0, 0.16, 0.4, 0.09), note(440.0, 0.28, 0.4, 0.16)].concat(),
            Self::MessageIn => [note(880.0, 0.09, 0.3, 0.05), note(1318.5, 0.16, 0.3, 0.08)].concat(),
            Self::MessageOut => note(1046.5, 0.12, 0.3, 0.05),
        }
    }

    pub fn wav(self) -> Vec<u8> {
        encode_wav(&self.samples())
    }
}

/// Paths of the written sound files.
#[derive(Debug, Clone)]
pub struct SoundFiles {
    dir: PathBuf,
}

impl SoundFiles {
    pub fn path(&self, sound: Sound) -> PathBuf {
        self.dir.join(sound.file_name())
    }
}

/// Writes every sound into `dir` unless an identical file is already there.
pub fn install(dir: &Path) -> io::Result<SoundFiles> {
    std::fs::create_dir_all(dir)?;
    for sound in Sound::ALL {
        let path = dir.join(sound.file_name());
        let bytes = sound.wav();
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(&path, bytes)?;
        }
    }
    Ok(SoundFiles { dir: dir.to_path_buf() })
}

/// A marimba-like arpeggio (E5 G#5 B5 E6, twice) followed by a pause.
fn ringtone() -> Vec<f32> {
    let mut out = Vec::new();
    for _ in 0..2 {
        for freq in [659.25, 830.61, 987.77, 1318.51] {
            out.extend(marimba(freq, 0.14));
        }
        out.extend(silence(0.12));
    }
    out.extend(silence(1.6));
    out
}

/// India's ringback: 400 Hz modulated at 25 Hz, 0.4 s on, 0.2 s off,
/// 0.4 s on, 2 s off.
fn ringback() -> Vec<f32> {
    let tone = |secs: f32| -> Vec<f32> {
        let n = (secs * RATE as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let carrier = (TAU * 400.0 * t).sin();
                let modulation = 0.5 + 0.5 * (TAU * 25.0 * t).sin();
                0.3 * carrier * modulation * edge(i, n)
            })
            .collect()
    };
    [tone(0.4), silence(0.2), tone(0.4), silence(2.0)].concat()
}

/// Segments of tone (Some(freq)) or silence, at `level`.
fn cadence(segments: &[(f32, Option<f32>)], level: f32) -> Vec<f32> {
    segments
        .iter()
        .flat_map(|&(secs, freq)| match freq {
            Some(f) => {
                let n = (secs * RATE as f32) as usize;
                (0..n).map(|i| level * (TAU * f * i as f32 / RATE as f32).sin() * edge(i, n)).collect::<Vec<_>>()
            }
            None => silence(secs),
        })
        .collect()
}

/// A sine note with a short attack and an exponential decay.
fn note(freq: f32, secs: f32, level: f32, decay: f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.004).min(1.0);
            level * attack * (-t / decay).exp() * (TAU * freq * t).sin() * edge(i, n)
        })
        .collect()
}

/// A struck bar: fundamental plus a quickly fading 4th harmonic.
fn marimba(freq: f32, secs: f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.003).min(1.0);
            let body = (-t / 0.09).exp() * (TAU * freq * t).sin();
            let click = 0.35 * (-t / 0.02).exp() * (TAU * freq * 4.0 * t).sin();
            0.55 * attack * (body + click) * edge(i, n)
        })
        .collect()
}

fn silence(secs: f32) -> Vec<f32> {
    vec![0.0; (secs * RATE as f32) as usize]
}

/// A 3 ms fade at both ends so segments start and stop without clicks.
fn edge(i: usize, n: usize) -> f32 {
    let ramp = (RATE as usize * 3 / 1000).max(1);
    let from_end = n - 1 - i.min(n - 1);
    (i.min(from_end) as f32 / ramp as f32).min(1.0)
}

fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duration(sound: Sound) -> f32 {
        sound.samples().len() as f32 / RATE as f32
    }

    #[test]
    fn sounds_are_well_formed() {
        for sound in Sound::ALL {
            let samples = sound.samples();
            assert!(!samples.is_empty(), "{sound:?}");
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.1 && peak <= 0.95, "{sound:?} peak {peak}");
            // Clean ends: no click at the start or end.
            assert!(samples[0].abs() < 0.01 && samples[samples.len() - 1].abs() < 0.01, "{sound:?}");
            let wav = sound.wav();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(wav.len(), 44 + samples.len() * 2);
        }
    }

    #[test]
    fn cadences_have_the_right_length() {
        assert!((duration(Sound::Ringback) - 3.0).abs() < 0.01);
        assert!((duration(Sound::CallWaiting) - 0.6).abs() < 0.01);
        assert!(duration(Sound::Ringtone) > 2.5 && duration(Sound::Ringtone) < 4.0);
        assert!(duration(Sound::Hangup) < 0.6);
    }

    #[test]
    fn install_writes_and_keeps_files() {
        let dir = std::env::temp_dir().join(format!("rustone-sounds-test-{}", std::process::id()));
        let files = install(&dir).unwrap();
        let path = files.path(Sound::Ringtone);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        install(&dir).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified, "identical file is left alone");
        let _ = std::fs::remove_dir_all(dir);
    }
}
