use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::protocol::{self, Command, Event, Fit, Limits, Monitor, Pause, Show};

pub const USAGE: &str = "\
usage:
  apolline-wall daemon                      run on its own and restore the last wallpapers
  apolline-wall set PATH [--output NAME] [--fit cover|contain|blur] [--transition MS]
  apolline-wall pause [--output NAME]
  apolline-wall resume [--output NAME]
  apolline-wall limits [--fps 1-240] [--scale 0.1-1]
  apolline-wall next|previous [--output NAME]
  apolline-wall stop-playlist [--output NAME]
  apolline-wall policy [--battery keep|reduce|pause] [--lock true|false] [--sleep true|false] [--fullscreen true|false]
  apolline-wall session locked|unlocked       report shell lock state
  apolline-wall status

PATH is a picture, a video (mp4, webm, mkv, mov, gif, ...) or a Shadertoy shader (.frag, .glsl).
Without arguments it reads JSON commands on stdin, which is how Apolline runs it.";

const WAIT: Duration = Duration::from_secs(20);

pub fn run(args: &[String]) -> Result<(), String> {
    let (verb, rest) = args.split_first().ok_or(USAGE)?;
    let options = Options::parse(rest)?;
    match verb.as_str() {
        "set" => {
            options.only(&["output", "fit", "transition"])?;
            set(&options)
        }
        "pause" | "resume" => {
            options.only(&["output"])?;
            options.no_path()?;
            send(&Command::Pause(Pause {
                output: options.get("output"),
                paused: verb == "pause",
            }))
        }
        "limits" => {
            options.only(&["fps", "scale"])?;
            options.no_path()?;
            let limits = Command::Limits(Limits {
                fps: options.number("fps")?,
                scale: options.number("scale")?,
            });
            protocol::parse(&protocol::line(&limits))?;
            send(&limits)
        }
        "next" | "previous" => {
            options.only(&["output"])?;
            options.no_path()?;
            send(&Command::Step(protocol::Step {
                output: options.get("output"),
                previous: verb == "previous",
            }))
        }
        "stop-playlist" => {
            options.only(&["output"])?;
            options.no_path()?;
            send(&Command::Playlist(protocol::Playlist {
                output: options.get("output"),
                sources: Vec::new(),
                settings: Default::default(),
                interval_secs: 300,
                fit: Fit::Cover,
            }))
        }
        "policy" => {
            options.only(&["battery", "lock", "sleep", "fullscreen"])?;
            options.no_path()?;
            let Event::Status { power, .. } = status(&mut connect()?)? else {
                unreachable!()
            };
            let mut policy = power.policy;
            if options.named.is_empty() {
                println!(
                    "{}",
                    toml::to_string(&policy).map_err(|err| err.to_string())?
                );
                return Ok(());
            }
            if let Some(mode) = options.get("battery") {
                policy.battery = match mode.as_str() {
                    "keep" => apolline_wall_protocol::power::BatteryMode::Keep,
                    "reduce" => apolline_wall_protocol::power::BatteryMode::Reduce,
                    "pause" => apolline_wall_protocol::power::BatteryMode::Pause,
                    _ => return Err("--battery is keep, reduce or pause".into()),
                };
            }
            if let Some(value) = options.number("lock")? {
                policy.pause_on_lock = value;
            }
            if let Some(value) = options.number("sleep")? {
                policy.pause_on_sleep = value;
            }
            if let Some(value) = options.number("fullscreen")? {
                policy.pause_on_fullscreen = value;
            }
            send(&Command::Policy(policy))
        }
        "session" => {
            options.only(&[])?;
            let locked = match options.path.as_deref() {
                Some("locked") => true,
                Some("unlocked") => false,
                _ => return Err("session requires locked or unlocked".into()),
            };
            send(&Command::Session(protocol::Session { locked }))
        }
        "status" => {
            options.only(&[])?;
            options.no_path()?;
            print!("{}", describe(&status(&mut connect()?)?));
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

struct Options {
    path: Option<String>,
    named: HashMap<String, String>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Options {
            path: None,
            named: HashMap::new(),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.strip_prefix("--") {
                Some(name) => {
                    let value = args.next().ok_or(format!("--{name} needs a value"))?;
                    options.named.insert(name.to_owned(), value.clone());
                }
                None if options.path.is_none() => options.path = Some(arg.clone()),
                None => return Err(format!("unexpected {arg}")),
            }
        }
        Ok(options)
    }

    fn only(&self, known: &[&str]) -> Result<(), String> {
        match self
            .named
            .keys()
            .find(|name| !known.contains(&name.as_str()))
        {
            Some(name) => Err(format!("unknown option --{name}\n\n{USAGE}")),
            None => Ok(()),
        }
    }

    fn no_path(&self) -> Result<(), String> {
        match &self.path {
            Some(path) => Err(format!("unexpected {path}")),
            None => Ok(()),
        }
    }

    fn get(&self, name: &str) -> Option<String> {
        self.named.get(name).cloned()
    }

    fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        self.named
            .get(name)
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| format!("--{name} {value} is not a number"))
            })
            .transpose()
    }
}

fn set(options: &Options) -> Result<(), String> {
    let path = options.path.as_deref().ok_or(USAGE)?;
    let source = std::fs::canonicalize(path).map_err(|err| format!("{path}: {err}"))?;
    let fit = match options.get("fit").as_deref() {
        None | Some("cover") => Fit::Cover,
        Some("contain") => Fit::Contain,
        Some("blur") => Fit::Blur,
        Some(other) => return Err(format!("--fit is cover, contain or blur, not {other}")),
    };
    let output = options.get("output");
    let mut stream = connect()?;
    let Event::Status { outputs, .. } = status(&mut stream)? else {
        unreachable!("status answers with a status")
    };
    let mut waiting: Vec<String> = outputs
        .iter()
        .filter(|monitor| output.as_ref().is_none_or(|name| *name == monitor.name))
        .filter(|monitor| monitor.source.as_ref() != Some(&source))
        .map(|monitor| monitor.name.clone())
        .collect();
    if let Some(name) = &output
        && !outputs.iter().any(|monitor| monitor.name == *name)
    {
        let names: Vec<&str> = outputs.iter().map(|m| m.name.as_str()).collect();
        return Err(format!(
            "no monitor called {name}; there is {}",
            names.join(", ")
        ));
    }
    write(
        &mut stream,
        &Command::Show(Show {
            properties: Default::default(),
            output: output.clone(),
            source: source.clone(),
            fit,
            transition_ms: options.number("transition")?.unwrap_or(400),
            look: None,
        }),
    )?;
    let mut failed = Vec::new();
    let deadline = Instant::now() + WAIT;
    let mut reader = BufReader::new(stream);
    while !waiting.is_empty() && Instant::now() < deadline {
        let (name, problem) = match next_event(&mut reader)? {
            Some(Event::Ready {
                output, source: s, ..
            }) if s == source => (output, None),
            Some(Event::Error {
                output: Some(output),
                source: Some(s),
                message,
            }) if s == source => (output, Some(message)),
            _ => continue,
        };
        waiting.retain(|waiting| *waiting != name);
        if let Some(problem) = problem {
            failed.push(problem);
        }
    }
    if let Some(problem) = failed.first() {
        return Err(problem.clone());
    }
    if !waiting.is_empty() {
        return Err(format!("{} didn't answer in time", waiting.join(", ")));
    }
    Ok(())
}

fn connect() -> Result<UnixStream, String> {
    let stream = UnixStream::connect(protocol::client_socket_path()).map_err(|_| {
        "apolline-wall isn't running: start it with `apolline-wall daemon`, or log in to Apolline"
            .to_owned()
    })?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|err| err.to_string())?;
    Ok(stream)
}

fn send(command: &Command) -> Result<(), String> {
    write(&mut connect()?, command)
}

fn write(stream: &mut UnixStream, command: &Command) -> Result<(), String> {
    writeln!(stream, "{}", protocol::line(command)).map_err(|err| err.to_string())
}

fn status(stream: &mut UnixStream) -> Result<Event, String> {
    write(stream, &Command::Status {})?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|err| err.to_string())?);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match next_event(&mut reader)? {
            Some(status @ Event::Status { .. }) => return Ok(status),
            Some(Event::Error {
                output: None,
                source: None,
                message,
            }) => return Err(message),
            _ => continue,
        }
    }
    Err("apolline-wall didn't answer".into())
}

/// One event, or None when nothing arrived before the read timeout.
fn next_event(reader: &mut impl BufRead) -> Result<Option<Event>, String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err("apolline-wall went away".into()),
        Ok(_) => Ok(serde_json::from_str(&line).ok()),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(err) => Err(err.to_string()),
    }
}

fn describe(status: &Event) -> String {
    let Event::Status {
        outputs,
        fps,
        scale,
        ..
    } = status
    else {
        return String::new();
    };
    let width = outputs.iter().map(|m| m.name.len()).max().unwrap_or(0);
    let mut text = String::new();
    for Monitor {
        name,
        source,
        paused,
        pause_reason,
        playlist_status,
        ..
    } in outputs
    {
        let shown = source.as_deref().map_or("nothing".to_owned(), |path| {
            home(path).display().to_string()
        });
        let paused = if *paused {
            pause_reason
                .as_ref()
                .map_or("  (paused)".to_owned(), |reason| {
                    format!("  (paused: {reason})")
                })
        } else {
            String::new()
        };
        let playlist = playlist_status
            .as_ref()
            .map_or(String::new(), |status| format!("  ({status})"));
        text.push_str(&format!("{name:width$}  {shown}{paused}{playlist}\n"));
    }
    let scale = scale.map_or("about 1080p".to_owned(), |scale| format!("{scale}"));
    text.push_str(&format!("shaders: {fps} fps, scale {scale}\n"));
    text
}

fn home(path: &Path) -> PathBuf {
    std::env::var_os("HOME")
        .and_then(|home| path.strip_prefix(home).ok())
        .map_or(path.to_path_buf(), |rest| Path::new("~").join(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(Into::into).collect()
    }

    #[test]
    fn options_are_a_path_and_named_values() {
        let options = Options::parse(&args("/w/fog.jpg --output DP-1 --fit contain")).unwrap();
        assert_eq!(options.path.as_deref(), Some("/w/fog.jpg"));
        assert_eq!(options.get("output").as_deref(), Some("DP-1"));
        assert!(options.only(&["output", "fit"]).is_ok());
        assert!(options.only(&["output"]).unwrap_err().contains("--fit"));
        assert!(Options::parse(&args("--fps")).is_err());
        assert!(Options::parse(&args("a b")).is_err());
    }

    #[test]
    fn bad_input_is_refused_before_anything_is_sent() {
        assert!(run(&args("limits --fps 0")).unwrap_err().contains("fps 0"));
        assert!(
            run(&args("limits --fps fast"))
                .unwrap_err()
                .contains("not a number")
        );
        assert!(
            run(&args("set /nowhere/fog.jpg"))
                .unwrap_err()
                .contains("/nowhere/fog.jpg")
        );
        assert!(run(&args("dance")).unwrap_err().contains("unknown command"));
    }

    #[test]
    fn status_reads_like_a_table() {
        let status = Event::Status {
            power: Default::default(),
            outputs: vec![
                Monitor {
                    name: "DP-1".into(),
                    source: Some("/w/fog.jpg".into()),
                    paused: false,
                    native_scene: false,
                    animated_scene: false,
                    manual_paused: false,
                    pause_reason: None,
                    playlist: false,
                    playlist_status: None,
                    look: None,
                },
                Monitor {
                    name: "HDMI-A-1".into(),
                    source: None,
                    paused: true,
                    native_scene: false,
                    animated_scene: false,
                    manual_paused: false,
                    pause_reason: None,
                    playlist: false,
                    playlist_status: None,
                    look: None,
                },
            ],
            fps: 30,
            scale: None,
        };
        assert_eq!(
            describe(&status),
            "DP-1      /w/fog.jpg\nHDMI-A-1  nothing  (paused)\nshaders: 30 fps, scale about 1080p\n"
        );
    }
}
