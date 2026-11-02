use std::time::Duration;

use apolline_wall_protocol::protocol::{self, Command, Event};
use iced::Subscription;
use iced::futures::{SinkExt, Stream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub const NOT_RUNNING: &str = "apolline-wall isn't running";

#[derive(Clone, Debug)]
pub enum Update {
    Event(Event),
    Gone,
}

/// Everything the daemon says, starting with its status, for as long as it runs; it is looked
/// for again every two seconds while it doesn't.
pub fn watch() -> Subscription<Update> {
    Subscription::run(listen)
}

fn listen() -> impl Stream<Item = Update> {
    iced::stream::channel(32, async |mut output| {
        loop {
            if let Ok(stream) = UnixStream::connect(protocol::socket_path()).await {
                let (read, mut write) = stream.into_split();
                let status = format!("{}\n", protocol::line(&Command::Status {}));
                if write.write_all(status.as_bytes()).await.is_ok() {
                    let mut lines = BufReader::new(read).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(event) = serde_json::from_str::<Event>(&line)
                            && output.send(Update::Event(event)).await.is_err()
                        {
                            return;
                        }
                    }
                }
            }
            if output.send(Update::Gone).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

/// Sends a command and answers with the daemon's status after it, or with why it couldn't.
pub async fn ask(command: Command) -> Result<Event, String> {
    ask_at(&protocol::socket_path(), command).await
}

async fn ask_at(socket: &std::path::Path, command: Command) -> Result<Event, String> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|_| NOT_RUNNING.to_owned())?;
    let (read, mut write) = stream.into_split();
    let text = format!(
        "{}\n{}\n",
        protocol::line(&command),
        protocol::line(&Command::Status {})
    );
    write
        .write_all(text.as_bytes())
        .await
        .map_err(|err| err.to_string())?;
    let answer = async {
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<Event>(&line) {
                Ok(status @ Event::Status { .. }) => return Ok(status),
                Ok(Event::Error {
                    output: None,
                    source: None,
                    message,
                }) => return Err(message),
                _ => continue,
            }
        }
        Err("apolline-wall went away".to_owned())
    };
    tokio::time::timeout(Duration::from_secs(5), answer)
        .await
        .unwrap_or_else(|_| Err("apolline-wall didn't answer".into()))
}

/// Starts the daemon on its own, outliving this window.
pub fn start() -> Result<(), String> {
    std::process::Command::new("apolline-wall")
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|err| format!("apolline-wall daemon: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    async fn answering(reply: &'static str) -> Result<Event, String> {
        let dir = std::env::temp_dir().join(format!(
            "apolline-ask-{}-{}",
            std::process::id(),
            reply.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("apolline-wall.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            let command = lines.next_line().await.unwrap().unwrap();
            let status = lines.next_line().await.unwrap().unwrap();
            assert!(command.starts_with("{\"pause\""), "{command}");
            assert_eq!(status, r#"{"status":{}}"#);
            write.write_all(reply.as_bytes()).await.unwrap();
        });
        let pause = Command::Pause(protocol::Pause {
            output: None,
            paused: true,
        });
        let answer = ask_at(&socket, pause).await;
        server.await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        answer
    }

    #[tokio::test]
    async fn a_command_is_answered_with_the_status_after_it_or_the_error() {
        let ready = "{\"ready\":{\"output\":\"DP-1\",\"source\":\"/w/a.jpg\",\"poster\":\"/w/a.jpg\"}}\n{\"status\":{\"outputs\":[],\"fps\":30}}\n";
        assert!(matches!(
            answering(ready).await,
            Ok(Event::Status { fps: 30, .. })
        ));
        let refused = "{\"error\":{\"message\":\"fps 0 is outside 1..240\"}}\n";
        assert_eq!(
            answering(refused).await,
            Err("fps 0 is outside 1..240".into())
        );
    }
}
