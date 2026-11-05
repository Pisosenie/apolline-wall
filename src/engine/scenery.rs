use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use apolline_wall_protocol::install::{self, Need};

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};

use super::process::Process;
use super::{App, Picture, Surface};
use crate::protocol::{self, Event, Fit, Show};

/// linux-wallpaperengine draws Wallpaper Engine's scenes and web wallpapers; another program
/// can stand in for it, as tests do.
const RENDERER: &str = "linux-wallpaperengine";
const OVERRIDE: &str = "APOLLINE_WALL_SCENE_RENDERER";

/// A wallpaper another program draws on its own layer surface while ours steps aside.
pub(super) struct External {
    process: Process,
    pub stopped: bool,
    /// What showed before, put back if the renderer gives up.
    pub fallback: Option<Show>,
}

pub(super) struct Ended {
    pub id: u64,
    pub why: String,
}

impl App {
    pub(super) fn external_pids(&self) -> Vec<u32> {
        self.screens
            .iter()
            .filter_map(
                |screen| match screen.shown.as_ref().map(|shown| &shown.picture) {
                    Some(Picture::External(external)) => Some(external.process.pid),
                    _ => None,
                },
            )
            .collect()
    }

    /// Hands a scene the native renderer can't draw to linux-wallpaperengine. It draws with
    /// the scene's own property values: a saved edit it can't take doesn't keep it from showing.
    /// `reason` says why the native renderer passed, for when this fails too.
    pub(super) fn start_external(&mut self, index: usize, reason: String) {
        let Some(loading) = self.screens[index].loading.take() else {
            return;
        };
        let output = self.screens[index].name.clone();
        // Why it isn't drawn here, for the journal: what a scene needs that isn't done yet.
        eprintln!(
            "apolline-wall: {} goes to linux-wallpaperengine: {reason}",
            loading.show.source.display()
        );
        let fallback = self.screens[index].shown.as_ref().map(|shown| Show {
            output: Some(output.clone()),
            ..shown.show.clone()
        });
        match spawn(
            &output,
            &loading.show,
            self.effective_fps(),
            self.endings.clone(),
        ) {
            Ok(process) => {
                let id = process.id;
                let source = loading.show.source.clone();
                self.present(
                    index,
                    Picture::External(External {
                        process,
                        stopped: false,
                        fallback,
                    }),
                    loading,
                );
                self.hide(index);
                self.sync_externals();
                protocol::emit(&self.status());
                let timer = Timer::from_duration(Duration::from_millis(1500));
                let _ = self
                    .handle
                    .insert_source(timer, move |_, _, app: &mut App| {
                        app.announce_external(&output, id, &source);
                        TimeoutAction::Drop
                    });
            }
            Err(message) => {
                self.give_up(index, loading.show.clone());
                protocol::emit(&Event::Error {
                    output: Some(output),
                    source: Some(loading.show.source),
                    message: format!("Native scene: {reason}. {message}"),
                });
            }
        }
    }

    /// Says the scene is up once it has survived its start.
    fn announce_external(&mut self, output: &str, id: u64, source: &Path) {
        if self.external_at(id).is_some() {
            protocol::emit(&Event::Ready {
                output: output.to_owned(),
                source: source.to_path_buf(),
                poster: preview(source).unwrap_or_else(|| source.to_path_buf()),
            });
        }
    }

    pub(super) fn ended(&mut self, ended: Ended) {
        let Some(index) = self.external_at(ended.id) else {
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
            && self.rotation_key(&output).is_none()
        {
            self.selection.set(fallback);
            self.refresh();
        }
        protocol::emit(&self.status());
    }

    fn external_at(&self, id: u64) -> Option<usize> {
        self.screens.iter().position(|screen| {
            matches!(
                screen.shown.as_ref().map(|shown| &shown.picture),
                Some(Picture::External(external)) if external.process.id == id
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
                if external.process.signal(signal).is_ok() {
                    external.stopped = paused;
                }
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
    let _ = external.process.signal(libc::SIGCONT);
    let _ = external.process.signal(libc::SIGTERM);
}

fn spawn(output: &str, show: &Show, fps: u32, ended: Sender<Ended>) -> Result<Process, String> {
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
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
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
    let process = match Process::open(child.id()) {
        Ok(process) => process,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "cannot track scene renderer safely: {err}; Linux 5.3 or newer with pidfd support is required"
            ));
        }
    };
    let exit = match process.exit_fd() {
        Ok(exit) => exit,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("cannot watch scene renderer exit: {err}"));
        }
    };
    let id = process.id;
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let said = match stderr {
            Some(mut stderr) => match super::diagnostics::collect(&exit, &mut stderr) {
                Ok(tail) => tail,
                Err(err) => {
                    let _ = child.kill();
                    format!("cannot read renderer diagnostics: {err}")
                }
            },
            None => String::new(),
        };
        let status = child
            .wait()
            .map(|status| status.to_string())
            .unwrap_or_default();
        let why = if said.is_empty() {
            status
        } else {
            format!("{status}: {said}")
        };
        let _ = ended.send(Ended { id, why });
    });
    Ok(process)
}

/// The still Wallpaper Engine keeps next to a project.
fn preview(project: &Path) -> Option<PathBuf> {
    let json: serde_json::Value = apolline_wall_protocol::source::read_project(project).ok()?;
    apolline_wall_protocol::source::project_file(project, json.get("preview")?.as_str()?)
}
