//! Equaliser feed: cava capturing the default sink through PipeWire, four
//! bands as raw ASCII levels on stdout, read on a thread and pushed into
//! the event loop. Runs while a player is around (cava sleeps by itself
//! after 3 s of silence, and keeps its auto-sensitivity across pauses).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::thread;

use smithay_client_toolkit::reexports::calloop::channel::Sender;

pub const BANDS: usize = 4;
const MAX_RANGE: u32 = 100;
const FRAMERATE: u32 = 30;

pub struct Cava {
    child: Option<Child>,
}

impl Cava {
    /// Start cava; every frame arrives as four levels 0..1.
    pub fn start(tx: Sender<[f32; BANDS]>) -> std::io::Result<Cava> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let conf = dir.join("island-shell-cava.conf");
        // With `method = pipewire`, cava 0.10's "auto" captures the default
        // sink's monitor and follows it when the default sink changes. An
        // explicit node name (the sink or "<sink>.monitor") does not resolve
        // and silently falls back to the default source: the microphone.
        let mut f = std::fs::File::create(&conf)?;
        write!(
            f,
            "[general]\nbars = {BANDS}\nframerate = {FRAMERATE}\nautosens = 1\nsensitivity = 250\n\
             lower_cutoff_freq = 50\nhigher_cutoff_freq = 10000\nsleep_timer = 3\n\
             [input]\nmethod = pipewire\nsource = auto\n\
             [output]\nmethod = raw\nraw_target = /dev/stdout\ndata_format = ascii\n\
             ascii_max_range = {MAX_RANGE}\nbar_delimiter = 59\nchannels = mono\nmono_option = average\n\
             [smoothing]\nnoise_reduction = 60\n"
        )?;
        drop(f);
        let mut child = Command::new("cava")
            .arg("-p")
            .arg(&conf)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("cava stdout");
        thread::Builder::new()
            .name("cava".into())
            .spawn(move || {
                let mut last = [0f32; BANDS];
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let mut levels = [0f32; BANDS];
                    for (i, v) in line.split(';').filter(|s| !s.is_empty()).take(BANDS).enumerate() {
                        levels[i] = v.parse::<f32>().unwrap_or(0.0) / MAX_RANGE as f32;
                    }
                    // silence repeats the same zeros: no wake-ups for those
                    if levels != last {
                        last = levels;
                        if tx.send(levels).is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("spawn cava reader");
        Ok(Cava { child: Some(child) })
    }
}

impl Drop for Cava {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            std::thread::spawn(move || { let _ = child.wait(); });
        }
    }
}
