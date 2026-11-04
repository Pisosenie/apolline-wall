use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use apolline_wall_protocol::install::{self, Need};

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};

use super::{App, Picture, Surface};
use crate::protocol::{self, Event, Fit, Show};

/// linux-wallpaperengine draws Wallpaper Engine's scenes and web wallpapers; another program
/// can stand in for it, as tests do.
const RENDERER: &str = "linux-wallpaperengine";
const OVERRIDE: &str = "APOLLINE_WALL_SCENE_RENDERER";

/// A wallpaper another program draws on its own layer surface while ours steps aside.
pub(super) struct External {
    pub pid: i32,
    pub stopped: bool,
    /// What showed before, put back if the renderer gives up.
    pub fallback: Option<Show>,
}

pub(super) struct Ended {
    pub pid: i32,
    pub why: String,
}

impl App {
    pub(super) fn start_external(&mut self, index: usize) {
        let Some(loading) = self.screens[index].loading.take() else {
            return;
        };
        let output = self.screens[index].name.clone();
        let fallback = self.screens[index].shown.as_ref().map(|shown| Show {
            output: Some(output.clone()),
            ..shown.show.clone()
        });
        match spawn(&output, &loading.show, self.fps, self.endings.clone()) {
            Ok(pid) => {
                let source = loading.show.source.clone();
                self.present(
                    index,
                    Picture::External(External {
                        pid,
                        stopped: false,
                        fallback,
                    }),
                    loading,
                );
                self.hide(index);
                self.sync_externals();
                let timer = Timer::from_duration(Duration::from_millis(1500));
                let _ = self
                    .handle
                    .insert_source(timer, move |_, _, app: &mut App| {
                        app.announce_external(&output, pid, &source);
                        TimeoutAction::Drop
                    });
            }
            Err(message) => {
                self.give_up(index, loading.show.clone());
                protocol::emit(&Event::Error {
                    output: Some(output),
                    source: Some(loading.show.source),
                    message,
                });
            }
        }
    }

    /// Says the scene is up once it has survived its start.
    fn announce_external(&mut self, output: &str, pid: i32, source: &Path) {
        if self.external_at(pid).is_some() {
            protocol::emit(&Event::Ready {
                output: output.to_owned(),
                source: source.to_path_buf(),
                poster: preview(source).unwrap_or_else(|| source.to_path_buf()),
            });
        }
    }

    pub(super) fn ended(&mut self, ended: Ended) {
        let Some(index) = self.external_at(ended.pid) else {
            return;
        };
        let Some(shown) = self.screens[index].shown.take() else {
            return;
        };
        let output = self.screens[index].name.clone();
        self.give_up(index, shown.show.clone());
        self.unhide(index);
        protocol::emit(&Event::Error {
            output: Some(output.clone()),
            source: Some(shown.show.source),
            message: format!("{RENDERER} stopped: {}", ended.why),
        });
        if let Picture::External(External {
            fallback: Some(fallback),
            ..
        }) = shown.picture
        {
            self.per_output.insert(output, fallback);
            self.refresh();
        }
    }

    fn external_at(&self, pid: i32) -> Option<usize> {
        self.screens.iter().position(|screen| {
            matches!(
                screen.shown.as_ref().map(|shown| &shown.picture),
                Some(Picture::External(external)) if external.pid == pid
            )
        })
    }

    /// Stops a scene's renderer while its monitor is paused and wakes it after.
    pub(super) fn sync_externals(&mut self) {
        for index in 0..self.screens.len() {
            let paused = self.is_paused(&self.screens[index].name);
            if let Some(shown) = &mut self.screens[index].shown
                && let Picture::External(external) = &mut shown.picture
                && external.stopped != paused
            {
                let signal = if paused { libc::SIGSTOP } else { libc::SIGCONT };
                unsafe { libc::kill(external.pid, signal) };
                external.stopped = paused;
            }
        }
    }

    pub(super) fn close_externals(&mut self) {
        for screen in &self.screens {
            if let Some(shown) = &screen.shown
                && let Picture::External(external) = &shown.picture
            {
                stop(external);
            }
        }
    }

    /// Takes our surface away so the renderer's is what shows.
    fn hide(&mut self, index: usize) {
        drop(self.screens[index].target.take());
        match &self.screens[index].surface {
            Surface::Wayland(_) => self.wayland_hide(index),
            Surface::X11(window) => self.x11_hide(window.id()),
        }
        self.screens[index].hidden = true;
    }

    pub(super) fn unhide(&mut self, index: usize) {
        if !self.screens[index].hidden {
            return;
        }
        drop(self.screens[index].target.take());
        let x11 = match &self.screens[index].surface {
            Surface::Wayland(_) => None,
            Surface::X11(window) => Some(window.id()),
        };
        match x11 {
            None => self.wayland_reopen(index),
            Some(id) => self.x11_show(id),
        }
        let screen = &mut self.screens[index];
        screen.frame_pending = false;
        screen.hidden = false;
        if x11.is_some() {
            self.place(index);
        }
    }
}

pub(super) fn stop(external: &External) {
    unsafe {
        libc::kill(external.pid, libc::SIGCONT);
        libc::kill(external.pid, libc::SIGTERM);
    }
}

fn spawn(output: &str, show: &Show, fps: u32, ended: Sender<Ended>) -> Result<i32, String> {
    let program = std::env::var_os(OVERRIDE).unwrap_or_else(|| RENDERER.into());
    let scaling = match show.fit {
        Fit::Cover => "fill",
        Fit::Contain => "fit",
    };
    let mut command = Command::new(&program);
    command
        .args(["--screen-root", output, "--bg"])
        .arg(&show.source)
        .args(["--scaling", scaling, "--fps", &fps.to_string()])
        .args(["--silent", "--disable-mouse"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // It goes when we go, however we go.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => format!(
            "Wallpaper Engine scenes need {RENDERER}: {}",
            install::hint(Need::SceneRenderer)
        ),
        _ => format!("{}: {err}", PathBuf::from(&program).display()),
    })?;
    let pid = child.id() as i32;
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let mut last: VecDeque<String> = VecDeque::new();
        for line in stderr
            .into_iter()
            .flat_map(|err| BufReader::new(err).lines().map_while(Result::ok))
        {
            if !line.trim().is_empty() {
                last.push_back(line);
                if last.len() > 3 {
                    last.pop_front();
                }
            }
        }
        let status = child
            .wait()
            .map(|status| status.to_string())
            .unwrap_or_default();
        let said = last.into_iter().collect::<Vec<_>>().join(" / ");
        let why = if said.is_empty() {
            status
        } else {
            format!("{status}: {said}")
        };
        let _ = ended.send(Ended { pid, why });
    });
    Ok(pid)
}

/// The still Wallpaper Engine keeps next to a project.
fn preview(project: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(project.join("project.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let preview = project.join(json.get("preview")?.as_str()?);
    preview.is_file().then_some(preview)
}
