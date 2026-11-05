use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use super::App;
use crate::protocol::{self, Command, Event};

/// Listens on the one socket per session; a live socket means another instance owns the wallpaper.
pub(super) fn serve(handle: &LoopHandle<'static, App>) -> Result<(), String> {
    apolline_wall_protocol::runtime::ensure()?;
    let path = protocol::socket_path();
    for live in std::iter::once(path.clone()).chain(protocol::old_socket_path()) {
        if UnixStream::connect(&live).is_ok() {
            return Err(format!(
                "another apolline-wall is already running on {}",
                live.display()
            ));
        }
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;
    let clients = handle.clone();
    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener, _| {
                while let Ok((stream, _)) = listener.accept() {
                    accept(&clients, stream);
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn accept(handle: &LoopHandle<'static, App>, stream: UnixStream) {
    let (Ok(listener), Ok(mut reply)) = (stream.try_clone(), stream.try_clone()) else {
        return;
    };
    for writer in [&listener, &reply] {
        let _ = writer.set_write_timeout(Some(Duration::from_millis(200)));
    }
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let Some(subscription) = protocol::listen(listener) else {
        return;
    };
    let mut pending = Vec::new();
    let _ = handle.insert_source(
        Generic::new(stream, Interest::READ, Mode::Level),
        move |_, stream, app: &mut App| {
            let _subscription = &subscription;
            let mut chunk = [0; 4096];
            let mut closed = false;
            loop {
                match (&**stream).read(&mut chunk) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(read) => {
                        if pending.len() + read > protocol::MAX_COMMAND_BYTES {
                            return Ok(PostAction::Remove);
                        }
                        pending.extend_from_slice(&chunk[..read]);
                        // Yield to other clients and rendering even if this peer keeps writing.
                        if pending.contains(&b'\n') {
                            break;
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => return Ok(PostAction::Remove),
                }
            }
            while let Some(end) = pending.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line);
                if line.trim().is_empty() {
                    continue;
                }
                match protocol::parse(line.trim()) {
                    Ok(Command::Status {}) => {
                        let _ = writeln!(reply, "{}", protocol::line(&app.status()));
                    }
                    Ok(command) => app.command(command),
                    Err(message) => {
                        let error = Event::Error {
                            output: None,
                            source: None,
                            message,
                        };
                        let _ = writeln!(reply, "{}", protocol::line(&error));
                    }
                }
            }
            Ok(if closed {
                PostAction::Remove
            } else {
                PostAction::Continue
            })
        },
    );
}
