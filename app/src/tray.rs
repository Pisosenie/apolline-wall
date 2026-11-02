use std::time::{SystemTime, UNIX_EPOCH};

use apolline_wall_protocol::protocol::{Command, Event, Monitor, Pause, Show};
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

enum Action {
    Pause(Option<String>, bool),
    Random(Option<String>),
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
                let all_paused = !outputs.is_empty() && outputs.iter().all(|m| m.paused);
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
    let paused = monitor.paused;
    let toggle = name.clone();
    SubMenu {
        label: format!("{} · {showing}", monitor.name),
        submenu: vec![
            item(if paused { "Resume" } else { "Pause" }, move |tray| {
                tray.act(Action::Pause(Some(toggle.clone()), !paused))
            }),
            item("Random wallpaper", move |tray| {
                tray.act(Action::Random(Some(name.clone())))
            }),
        ],
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
    let look = config
        .looks
        .get(&item.path.to_string_lossy().into_owned())
        .copied()
        .filter(|look| !look.is_plain());
    daemon::send(Command::Show(Show {
        output,
        source: item.path.clone(),
        fit: Default::default(),
        transition_ms: 600,
        look,
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
