//! What the daemon starts from: the display to draw on, the last session, and, for Apolline's
//! child mode, commands on stdin.

use std::io::BufRead;

use raw_window_handle::RawDisplayHandle;
use smithay_client_toolkit::reexports::calloop::LoopHandle;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use super::{App, Desktop, Persist, wayland, x11};
use crate::protocol::{self, Command, Event};
use crate::state;
use crate::video::Platform;

/// Wayland when there is one, X11 otherwise. A Wayland session that can't host a background,
/// GNOME without the Shell extension, still has Xwayland, whose desktop-type windows its
/// window manager keeps under everything.
pub(super) fn connect(
    handle: &LoopHandle<'static, App>,
) -> Result<(Desktop, RawDisplayHandle, Platform), String> {
    let x11 = std::env::var_os("DISPLAY").is_some();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match wayland::connect(handle) {
            Ok((wayland, display, platform)) => {
                return Ok((Desktop::Wayland(wayland), display, platform));
            }
            Err(why) if x11 => eprintln!("apolline-wall: {why}; drawing through Xwayland instead"),
            Err(why) => return Err(why),
        }
    }
    if x11 {
        let (x11, display, platform) = x11::connect(handle)?;
        return Ok((Desktop::X11(x11), display, platform));
    }
    Err("no display to draw on: neither WAYLAND_DISPLAY nor DISPLAY is set".into())
}

/// Replays the last session; a state file that doesn't parse is reported and left alone.
pub(super) fn restore(app: &mut App) -> Option<Persist> {
    let path = state::path()?;
    match state::load(&path) {
        Ok(saved) => {
            for command in saved.commands() {
                match command {
                    Command::Show(show) => match &show.output {
                        None => app.selection.everywhere = Some(show),
                        Some(output) => {
                            app.selection.per_output.insert(output.clone(), show);
                        }
                    },
                    other => app.command(other),
                }
            }
            app.selection.inherited = saved
                .outputs
                .iter()
                .filter(|(name, entry)| {
                    entry.follows_default
                        && !saved.playlists.contains_key(*name)
                        && saved
                            .wallpaper
                            .as_ref()
                            .is_some_and(|default| default.source == entry.source)
                })
                .map(|(name, _)| name.clone())
                .collect();
            app.restore_rotation_positions();
            Some(Persist {
                path,
                good: saved.clone(),
                state: saved,
            })
        }
        Err(message) => {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message: format!("{message}; not saving until it's fixed"),
            });
            None
        }
    }
}

pub(super) fn read_commands(commands: Sender<Result<Command, String>>) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !line.trim().is_empty() && commands.send(protocol::parse(&line)).is_err() {
            break;
        }
    }
}
