//! What's playing, for scenes that listen: the default output's monitor, read raw through
//! `parec` (PulseAudio's recorder, which PipeWire serves as well) while a scene shown needs it.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::spectrum::{Analyzer, Bands, RATE};

const RECORDER: &str = "parec";
const OVERRIDE: &str = "APOLLINE_WALL_PAREC";
/// Samples a side read at a time: a new spectrum about every 11 ms.
const CHUNK: usize = 512;

pub(super) struct Listener {
    child: Child,
    bands: Arc<Mutex<Bands>>,
    reader: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start() -> Result<Self, String> {
        let program = std::env::var_os(OVERRIDE).unwrap_or_else(|| RECORDER.into());
        let mut command = Command::new(&program);
        command
            .args([
                "--device=@DEFAULT_MONITOR@",
                "--format=float32le",
                "--channels=2",
                "--latency-msec=20",
                "--client-name=apolline-wall",
            ])
            .arg(format!("--rate={RATE}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        super::process::dies_with_us(&mut command);
        let mut child = command.spawn().map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => "Scenes that move to music need parec: libpulse on \
                Arch, pulseaudio-utils on Debian, Ubuntu and Fedora."
                .to_owned(),
            _ => format!("{}: {err}", std::path::Path::new(&program).display()),
        })?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("parec gave no output".into());
        };
        let bands = Arc::new(Mutex::new([[0.0; crate::spectrum::BANDS]; 2]));
        let shared = Arc::clone(&bands);
        let reader = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                let mut analyzer = Analyzer::new();
                let mut bytes = vec![0u8; CHUNK * 2 * 4];
                let mut samples = vec![0.0f32; CHUNK * 2];
                let mut last = Instant::now();
                while out.read_exact(&mut bytes).is_ok() {
                    for (sample, raw) in samples.iter_mut().zip(bytes.as_chunks::<4>().0) {
                        *sample = f32::from_le_bytes(*raw);
                    }
                    analyzer.push(&samples);
                    let now = Instant::now();
                    let fresh = analyzer.bands(now.duration_since(last).as_secs_f32());
                    last = now;
                    *shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = fresh;
                }
            })
            .map_err(|err| {
                let _ = child.kill();
                let _ = child.wait();
                format!("audio: {err}")
            })?;
        Ok(Listener {
            child,
            bands,
            reader: Some(reader),
        })
    }

    /// The spectrum as it was last heard, or silence once parec has stopped.
    pub fn bands(&self) -> Bands {
        if self.reader.as_ref().is_none_or(JoinHandle::is_finished) {
            return [[0.0; crate::spectrum::BANDS]; 2];
        }
        *self
            .bands
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // With parec gone its output ends, and so does the reader.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
