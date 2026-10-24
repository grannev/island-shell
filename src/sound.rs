//! Short sounds: the timer ruler's ticks (the stand-in for the haptic
//! clicks Vorssaint's ruler gives on a MacBook's trackpad; this ThinkPad's
//! Sensel pad exposes no haptics to the system) and the timer's alarm.
//! They are played by `pw-play` on a worker thread, never on the UI thread;
//! ticks that pile up while one is playing collapse into one.
use std::path::PathBuf;
use std::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// a minute passing under the ruler's pointer
    Tick,
    /// a countdown or a Pomodoro phase has run out
    Alarm,
}

const ALARM: &str = "/usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga";
const RATE: u32 = 48_000;

pub struct Player {
    tx: mpsc::Sender<Sound>,
}

impl Player {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Sound>();
        let spawned = std::thread::Builder::new().name("island-sound".into()).spawn(move || {
            let tick = tick_file();
            while let Ok(first) = rx.recv() {
                // what queued up while the last sound played: one tick is
                // enough, an alarm is never dropped
                let mut tick_due = first == Sound::Tick;
                let mut alarm_due = first == Sound::Alarm;
                while let Ok(more) = rx.try_recv() {
                    match more {
                        Sound::Tick => tick_due = true,
                        Sound::Alarm => alarm_due = true,
                    }
                }
                if alarm_due {
                    // twice, as a kitchen timer rings
                    for _ in 0..2 { play(ALARM.as_ref(), "0.9"); }
                } else if tick_due {
                    if let Some(path) = &tick { play(path, "0.55"); }
                }
            }
        });
        if let Err(e) = spawned { eprintln!("island-shell: sound worker: {e}"); }
        Player { tx }
    }

    pub fn play(&self, sound: Sound) {
        let _ = self.tx.send(sound);
    }
}

fn play(path: &std::path::Path, volume: &str) {
    let status = std::process::Command::new("pw-play")
        .arg("--volume").arg(volume)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if status.is_err() {
        // no PipeWire tools: the PulseAudio client plays it instead
        let _ = std::process::Command::new("paplay")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// The tick, written once per session into the runtime directory.
fn tick_file() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join("island-shell-tick.wav");
    std::fs::write(&path, tick_wav()).ok()?;
    Some(path)
}

/// A soft, short click: two damped tones, a woody knock rather than a beep.
pub fn tick_wav() -> Vec<u8> {
    let samples = (RATE as f64 * 0.018) as usize;
    let mut pcm = Vec::with_capacity(samples * 2);
    for n in 0..samples {
        let t = n as f64 / RATE as f64;
        let attack = (t / 0.0004).min(1.0);
        let body = (std::f64::consts::TAU * 1700.0 * t).sin() * (-t / 0.0035).exp();
        let edge = (std::f64::consts::TAU * 4200.0 * t).sin() * (-t / 0.0012).exp();
        let v = attack * (0.55 * body + 0.3 * edge) * 0.5;
        pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * i16::MAX as f64) as i16).to_le_bytes());
    }
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_tick_is_a_short_quiet_wav() {
        let wav = super::tick_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let data = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data + 44, wav.len());
        let peak = wav[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 3000 && peak < 20000, "peak {peak}");
        assert!(data / 2 < 1200, "under 25 ms");
    }
}
