use std::time::{SystemTime, UNIX_EPOCH};

use apolline_wall_protocol::protocol::{Command, Event, Monitor, Pause, Playlist, Show, Step};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{MenuItem, TrayMethods};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::daemon::{self, Update};
use crate::library::{self, Item};
use crate::{config, engine};

/// A tray icon that lives without the window: pause, resume and a random wallpaper for each
/// monitor, and the window a click away.
pub fn run() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(1)
        .build()
        .map_err(|err| err.to_string())?;
    runtime.block_on(serve())
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Pause(Option<String>, bool),
    Random(Option<String>),
    Step(String, bool),
    StopPlaylist(String),
    Open,
}

struct Tray {
    outputs: Option<Vec<Monitor>>,
    actions: UnboundedSender<Action>,
}

async fn serve() -> Result<(), String> {
    let (actions, mut requested) = unbounded_channel();
    let tray = Tray {
        outputs: None,
        actions,
    };
    // At login the tray often starts before the bar that hosts it; it waits for the bar.
    let handle = tray
        .assume_sni_available(true)
        .spawn()
        .await
        .map_err(|err| format!("the tray: {err}"))?;
    let follower = handle.clone();
    tokio::spawn(daemon::follow(move |update| {
        let handle = follower.clone();
        async move {
            handle.update(|tray: &mut Tray| tray.heard(update)).await;
            !handle.is_closed()
        }
    }));
    while let Some(action) = requested.recv().await {
        let outputs = handle
            .update(|tray: &mut Tray| tray.outputs.clone().unwrap_or_default())
            .await
            .unwrap_or_default();
        let done = match action {
            Action::Pause(output, paused) => {
                daemon::send(Command::Pause(Pause { output, paused })).await
            }
            Action::Random(output) => random(output, &outputs).await,
            Action::Step(output, previous) => {
                daemon::send(Command::Step(Step {
                    output: Some(output),
                    previous,
                }))
                .await
            }
            Action::StopPlaylist(output) => {
                daemon::send(Command::Playlist(Playlist {
                    output: Some(output),
                    sources: Vec::new(),
                    settings: Default::default(),
                    interval_secs: 300,
                    fit: Default::default(),
                }))
                .await
            }
            Action::Open => open_window(),
        };
        if let Err(message) = done {
            eprintln!("apolline-wallpapers: {message}");
        }
    }
    Ok(())
}

impl Tray {
    fn heard(&mut self, update: Update) {
        match update {
            Update::Event(Event::Status { outputs, .. }) => self.outputs = Some(outputs),
            Update::Event(Event::Ready { output, source, .. }) => {
                if let Some(monitor) = self
                    .outputs
                    .iter_mut()
                    .flatten()
                    .find(|monitor| monitor.name == output)
                {
                    monitor.source = Some(source);
                }
            }
            Update::Event(Event::Error { .. }) => {}
            Update::Gone => self.outputs = None,
        }
    }

    fn act(&self, action: Action) {
        let _ = self.actions.send(action);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "apolline-wallpapers".into()
    }

    fn title(&self) -> String {
        "Wallpapers".into()
    }

    fn icon_name(&self) -> String {
        "preferences-desktop-wallpaper".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.act(Action::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu: Vec<MenuItem<Self>> = Vec::new();
        match &self.outputs {
            None => menu.push(
                StandardItem {
                    label: "apolline-wall isn't running".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ),
            Some(outputs) => {
                for monitor in outputs {
                    menu.push(monitor_menu(monitor));
                }
                let all_paused = !outputs.is_empty() && outputs.iter().all(|m| m.manual_paused);
                menu.push(MenuItem::Separator);
                menu.push(item(
                    if all_paused {
                        "Resume all"
                    } else {
                        "Pause all"
                    },
                    move |tray| tray.act(Action::Pause(None, !all_paused)),
                ));
                menu.push(item("Random on every monitor", |tray| {
                    tray.act(Action::Random(None))
                }));
            }
        }
        menu.push(MenuItem::Separator);
        menu.push(item("Open Wallpapers", |tray| tray.act(Action::Open)));
        menu.push(item("Quit", |_| std::process::exit(0)));
        menu
    }
}

fn monitor_menu(monitor: &Monitor) -> MenuItem<Tray> {
    let showing = monitor
        .source
        .as_deref()
        .map_or("nothing".to_owned(), engine::title);
    let name = monitor.name.clone();
    let paused = monitor.manual_paused;
    let toggle = name.clone();
    let mut submenu = vec![item(if paused { "Resume" } else { "Pause" }, move |tray| {
        tray.act(Action::Pause(Some(toggle.clone()), !paused))
    })];
    if let Some(reason) = &monitor.pause_reason {
        submenu.push(notice(reason));
    }
    if monitor.playlist {
        submenu.push(MenuItem::Separator);
        if let Some(status) = &monitor.playlist_status {
            submenu.push(notice(&status.to_string()));
        }
        let can_step = monitor
            .playlist_status
            .as_ref()
            .is_none_or(|status| status.can_step);
        let previous = name.clone();
        let next = name.clone();
        for (label, output, backwards) in [
            ("Previous wallpaper", previous, true),
            ("Next wallpaper", next, false),
        ] {
            let mut control = StandardItem {
                label: label.into(),
                enabled: can_step,
                ..Default::default()
            };
            control.activate =
                Box::new(move |tray: &mut Tray| tray.act(Action::Step(output.clone(), backwards)));
            submenu.push(control.into());
        }
        let stop = name.clone();
        let label = if monitor
            .playlist_status
            .as_ref()
            .is_some_and(|status| status.shared)
        {
            "Stop shared playlist"
        } else {
            "Stop playlist"
        };
        submenu.push(item(label, move |tray| {
            tray.act(Action::StopPlaylist(stop.clone()))
        }));
        if monitor
            .playlist_status
            .as_ref()
            .is_some_and(|status| status.exhausted())
        {
            submenu.push(notice("Choose a wallpaper or start the playlist again"));
        }
        submenu.push(MenuItem::Separator);
    }
    submenu.push(item("Random wallpaper", move |tray| {
        tray.act(Action::Random(Some(name.clone())))
    }));
    SubMenu {
        label: format!("{} · {showing}", monitor.name),
        submenu,
        ..Default::default()
    }
    .into()
}

fn notice(label: &str) -> MenuItem<Tray> {
    StandardItem {
        label: label.into(),
        enabled: false,
        ..Default::default()
    }
    .into()
}

fn item(label: &str, activate: impl Fn(&mut Tray) + Send + 'static) -> MenuItem<Tray> {
    StandardItem {
        label: label.into(),
        activate: Box::new(activate),
        ..Default::default()
    }
    .into()
}

/// Any wallpaper from the library that can be shown, other than what the monitor shows now.
async fn random(output: Option<String>, outputs: &[Monitor]) -> Result<(), String> {
    let config = config::load();
    let items = library::scan(config.all_folders(), engine::folders()).await;
    let showing: Vec<_> = outputs
        .iter()
        .filter(|monitor| output.as_ref().is_none_or(|name| *name == monitor.name))
        .filter_map(|monitor| monitor.source.clone())
        .collect();
    let choices: Vec<&Item> = items
        .iter()
        .filter(|item| item.what.missing().is_none() && !showing.contains(&item.path))
        .collect();
    let Some(item) = pick(&choices) else {
        return Err("no other wallpaper in the library to pick".into());
    };
    let settings = config.wallpaper_settings(&item.path);
    daemon::send(Command::Show(Show {
        properties: settings.properties,
        output,
        source: item.path.clone(),
        fit: Default::default(),
        transition_ms: 600,
        look: settings.look,
    }))
    .await
}

fn pick<'a>(choices: &[&'a Item]) -> Option<&'a Item> {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.subsec_nanos() as usize);
    choices.get(seed % choices.len().max(1)).copied()
}

fn open_window() -> Result<(), String> {
    let program = std::env::current_exe().map_err(|err| err.to_string())?;
    std::process::Command::new(program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|err| format!("opening the window: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(exhausted: bool) -> Monitor {
        let mut monitor: Monitor =
            serde_json::from_str(r#"{"name":"DP-1","paused":false,"playlist":true}"#).unwrap();
        monitor.playlist_status = Some(apolline_wall_protocol::playlist::Status {
            position: 2,
            total: 3,
            skipped: if exhausted { 3 } else { 1 },
            shared: true,
            suspended: false,
            can_step: !exhausted,
        });
        monitor
    }

    #[test]
    fn tray_steps_and_stop_target_the_selected_monitor() {
        let (actions, mut received) = unbounded_channel();
        let mut tray = Tray {
            outputs: None,
            actions,
        };
        let MenuItem::SubMenu(menu) = monitor_menu(&monitor(false)) else {
            panic!("expected monitor submenu")
        };
        let mut checked = 0;
        for item in menu.submenu {
            let MenuItem::Standard(item) = item else {
                continue;
            };
            let expected = match item.label.as_str() {
                "Previous wallpaper" => Action::Step("DP-1".into(), true),
                "Next wallpaper" => Action::Step("DP-1".into(), false),
                "Stop shared playlist" => Action::StopPlaylist("DP-1".into()),
                _ => continue,
            };
            checked += 1;
            assert!(item.enabled);
            (item.activate)(&mut tray);
            assert_eq!(received.try_recv().unwrap(), expected);
        }
        assert_eq!(checked, 3);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn an_exhausted_playlist_disables_steps_but_keeps_stop_available() {
        let MenuItem::SubMenu(menu) = monitor_menu(&monitor(true)) else {
            panic!("expected monitor submenu")
        };
        let controls: Vec<_> = menu
            .submenu
            .into_iter()
            .filter_map(|item| match item {
                MenuItem::Standard(item) => Some((item.label, item.enabled)),
                _ => None,
            })
            .collect();
        assert!(controls.contains(&("Next wallpaper".into(), false)));
        assert!(controls.contains(&("Previous wallpaper".into(), false)));
        assert!(controls.contains(&("Stop shared playlist".into(), true)));
        assert!(controls.contains(&("Shared playlist stopped: no playable entries".into(), false)));
    }
}
