mod browse;
mod config;
mod daemon;
mod download;
mod engine;
mod library;
mod presets;
mod preview;
mod properties;
mod report;
mod steamcmd;
mod style;
#[cfg(target_os = "linux")]
mod tray;
mod view;
mod workshop;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use apolline_wall_protocol::protocol::{
    Adjust, Command, Event, Fit, Limits, Look, Monitor, Pause, Show,
};
use iced::widget::image::Handle;
use iced::{Element, Font, Subscription, Task, Theme, window};

use crate::library::{Item, What};

const FONTS: [&[u8]; 3] = [
    include_bytes!("../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
];

/// Thumbnails made at once; decoding large pictures all together would spike memory.
const BUSY: usize = 4;

fn main() -> iced::Result {
    #[cfg(target_os = "linux")]
    if std::env::args().any(|arg| arg == "--tray") {
        if let Err(message) = tray::run() {
            eprintln!("apolline-wallpapers: {message}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let page = match std::env::args()
        .skip_while(|arg| arg != "--page")
        .nth(1)
        .as_deref()
    {
        Some("workshop") => Page::Workshop,
        Some("settings") => Page::Settings,
        _ => Page::Library,
    };
    let app = iced::application(move || App::new(page), App::update, App::view)
        .title("Wallpapers")
        .subscription(App::subscription)
        .theme(Theme::Dark)
        .default_font(Font::with_name(style::UI))
        .window(window::Settings {
            size: iced::Size::new(1240.0, 800.0),
            min_size: Some(iced::Size::new(860.0, 560.0)),
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: "apolline-wallpapers".into(),
                ..Default::default()
            },
            ..window::Settings::default()
        });
    FONTS
        .into_iter()
        .fold(app, |app, font| app.font(font))
        .run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Library,
    Workshop,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Favorites,
    Pictures,
    Videos,
    Shaders,
    Engine,
}

pub struct Daemon {
    pub power: apolline_wall_protocol::power::Status,
    pub outputs: Vec<Monitor>,
    pub fps: u32,
    pub scale: Option<f32>,
}

pub struct App {
    pub config: config::Config,
    pub items: Vec<Item>,
    pub thumbs: HashMap<PathBuf, Handle>,
    queue: VecDeque<Item>,
    busy: usize,
    pub selected: Option<PathBuf>,
    pub filter: Filter,
    pub search: String,
    pub target: Option<String>,
    pub fit: Fit,
    pub page: Page,
    pub daemon: Option<Daemon>,
    pub notice: Option<String>,
    /// What went wrong lately, newest last, for a problem report.
    errors: VecDeque<String>,
    /// What the next property preset will be called.
    pub preset_name: String,
    pub folder: String,
    pub fps: u32,
    pub scale: f32,
    pub browser: workshop::Browser,
    /// The look being edited for the picked wallpaper.
    pub look: Look,
    pub tray_at_login: bool,
    pub steam: download::Steam,
    pub preview: Option<preview::Preview>,
    pub property_editor: properties::Editor,
    preview_generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Tweak {
    Brightness(f32),
    Contrast(f32),
    Saturation(f32),
    Speed(f32),
    Flip(bool),
}

#[derive(Clone, Debug)]
pub enum Message {
    Scanned(Vec<Item>),
    Thumb(PathBuf, Option<Handle>),
    Select(PathBuf),
    Preview(preview::Action),
    Properties(properties::Action),
    Filter(Filter),
    Search(String),
    Target(Option<String>),
    Fit(Fit),
    Apply,
    Favorite(PathBuf),
    Rotate,
    RotationInterval(u32),
    Step(String, bool),
    StopPlaylist(String),
    Pause(Option<String>, bool),
    Daemon(daemon::Update),
    Answered(Result<Event, String>),
    /// The daemon's answer to a wallpaper applied with these properties, by its path.
    /// Saves a problem report about the picked wallpaper.
    Report,
    Preset(presets::Preset),
    Applied(
        String,
        apolline_scene::properties::Values,
        Result<Event, String>,
    ),
    Page(Page),
    Folder(String),
    AddFolder,
    RemoveFolder(PathBuf),
    Fps(u32),
    Scale(f32),
    Limit,
    Start,
    Power(view::power::Change),
    Rescan,
    Dismiss,
    Open(String),
    Shop(browse::Shop),
    Look(Tweak),
    ApplyLook,
    RevertLook,
    ResetLook,
    Sent(Result<(), String>),
    TrayAtLogin(bool),
    Steam(download::Act),
}

impl App {
    fn new(page: Page) -> (Self, Task<Message>) {
        let mut config = config::load();
        if config.rotation_minutes == 0 {
            config.rotation_minutes = 5;
        }
        let mut app = App {
            browser: workshop::Browser::new(config.questionable),
            look: Look::default(),
            tray_at_login: config::tray_at_login(),
            steam: download::Steam::new(config.steam_account.as_deref()),
            preview: None,
            property_editor: properties::Editor::default(),
            preview_generation: 0,
            config,
            items: Vec::new(),
            thumbs: HashMap::new(),
            queue: VecDeque::new(),
            busy: 0,
            selected: None,
            filter: Filter::All,
            search: String::new(),
            target: None,
            fit: Fit::Cover,
            page,
            daemon: None,
            notice: None,
            errors: VecDeque::new(),
            preset_name: String::new(),
            folder: String::new(),
            fps: 30,
            scale: 1.0,
        };
        let scan = app.rescan();
        let workshop = match page {
            Page::Workshop => app.open_workshop(),
            _ => Task::none(),
        };
        (app, Task::batch([scan, workshop]))
    }

    pub fn folders(&self) -> Vec<PathBuf> {
        self.config.all_folders()
    }

    pub fn rescan(&self) -> Task<Message> {
        Task::perform(
            library::scan(self.folders(), engine::folders()),
            Message::Scanned,
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if self.preview.is_some()
            && matches!(
                message,
                Message::Search(_) | Message::Select(_) | Message::Filter(_) | Message::Page(_)
            )
        {
            return Task::none();
        }
        match message {
            Message::Power(change) => {
                let Some(daemon) = &self.daemon else {
                    return Task::none();
                };
                let mut policy = daemon.power.policy.clone();
                match change {
                    view::power::Change::Lock(v) => policy.pause_on_lock = v,
                    view::power::Change::Sleep(v) => policy.pause_on_sleep = v,
                    view::power::Change::Fullscreen(v) => policy.pause_on_fullscreen = v,
                    view::power::Change::Battery(v) => policy.battery = v,
                }
                self.ask(Command::Policy(policy))
            }
            Message::Preview(action) => self.preview(action),
            Message::Properties(action) => self.properties(action),
            Message::Preset(preset) => self.preset(preset),
            Message::Scanned(items) => {
                self.queue = items
                    .iter()
                    .filter(|item| !self.thumbs.contains_key(&item.path))
                    .cloned()
                    .collect();
                let gone = self
                    .selected
                    .as_ref()
                    .is_none_or(|picked| !items.iter().any(|item| &item.path == picked));
                self.items = items;
                if gone {
                    let playable = self.items.iter().find(|item| item.what.missing().is_none());
                    if let Some(path) = playable
                        .or(self.items.first())
                        .map(|item| item.path.clone())
                    {
                        self.pick(path);
                    }
                }
                Task::batch([self.pump(), self.load_properties()])
            }
            Message::Thumb(path, handle) => {
                self.busy = self.busy.saturating_sub(1);
                if let Some(handle) = handle {
                    self.thumbs.insert(path, handle);
                }
                self.pump()
            }
            Message::Select(path) => {
                self.pick(path);
                self.load_properties()
            }
            Message::Favorite(path) => {
                if !self.config.favorites.remove(&path) {
                    self.config.favorites.insert(path);
                }
                self.notice = config::save(&self.config).err();
                Task::none()
            }
            Message::RotationInterval(minutes) => {
                self.config.rotation_minutes = minutes.clamp(1, 60);
                self.notice = config::save(&self.config).err();
                Task::none()
            }
            Message::Rotate => {
                let sources = self
                    .visible()
                    .into_iter()
                    .filter(|item| item.what.missing().is_none())
                    .map(|item| item.path.clone())
                    .collect::<Vec<_>>();
                if sources.is_empty() {
                    return Task::none();
                }
                self.ask(Command::Playlist(
                    apolline_wall_protocol::protocol::Playlist {
                        output: self.target.clone(),
                        settings: self.config.playlist_settings(&sources),
                        sources,
                        interval_secs: u64::from(self.config.rotation_minutes.clamp(1, 60)) * 60,
                        fit: self.fit,
                    },
                ))
            }
            Message::Step(output, previous) => {
                self.ask(Command::Step(apolline_wall_protocol::protocol::Step {
                    output: Some(output),
                    previous,
                }))
            }
            Message::StopPlaylist(output) => self.ask(Command::Playlist(
                apolline_wall_protocol::protocol::Playlist {
                    output: Some(output),
                    sources: Vec::new(),
                    settings: Default::default(),
                    interval_secs: 300,
                    fit: self.fit,
                },
            )),
            Message::Look(tweak) => {
                match tweak {
                    Tweak::Brightness(value) => self.look.brightness = value,
                    Tweak::Contrast(value) => self.look.contrast = value,
                    Tweak::Saturation(value) => self.look.saturation = value,
                    Tweak::Speed(value) => self.look.speed = value,
                    Tweak::Flip(on) => self.look.flip = on,
                }
                self.adjust(true)
            }
            Message::ApplyLook => {
                self.keep_look();
                self.adjust(false)
            }
            Message::RevertLook => {
                self.look = self
                    .selected
                    .as_deref()
                    .map(|path| self.saved_look(path))
                    .unwrap_or_default();
                self.adjust(true)
            }
            Message::ResetLook => {
                self.look = Look::default();
                self.adjust(true)
            }
            Message::TrayAtLogin(on) => {
                self.notice = config::set_tray_at_login(on).err();
                self.tray_at_login = config::tray_at_login();
                Task::none()
            }
            Message::Sent(result) => {
                self.notice = result.err();
                Task::none()
            }
            Message::Filter(filter) => {
                self.filter = filter;
                Task::none()
            }
            Message::Search(search) => {
                self.search = search;
                Task::none()
            }
            Message::Target(target) => {
                self.target = target;
                Task::none()
            }
            Message::Fit(fit) => {
                self.fit = fit;
                Task::none()
            }
            Message::Apply => match self
                .picked()
                .filter(|item| item.what.missing().is_none())
                .map(|item| {
                    (
                        item.path.clone(),
                        item.what.takes_a_look() || item.what == What::Scene,
                    )
                }) {
                Some((source, takes_a_look)) => {
                    let properties = if self.property_editor.path.as_ref() == Some(&source) {
                        if !self.property_editor.can_apply()
                            && let Some(error) = &self.property_editor.error
                        {
                            self.notice = Some(error.clone());
                            return Task::none();
                        }
                        self.property_editor.values.clone()
                    } else {
                        Default::default()
                    };
                    let key = source.to_string_lossy().to_string();
                    self.keep_look();
                    // Properties are kept only once the daemon has shown the wallpaper with
                    // them, so an edit it refuses doesn't follow the wallpaper into playlists.
                    let show = Command::Show(Show {
                        properties: properties.clone(),
                        output: self.target.clone(),
                        source,
                        fit: self.fit,
                        transition_ms: 600,
                        look: Some(self.look).filter(|look| takes_a_look && !look.is_plain()),
                    });
                    Task::perform(daemon::ask(show), move |answer| {
                        Message::Applied(key.clone(), properties.clone(), answer)
                    })
                }
                None => Task::none(),
            },
            Message::Pause(output, paused) => self.ask(Command::Pause(Pause { output, paused })),
            Message::Fps(fps) => {
                self.fps = fps;
                Task::none()
            }
            Message::Scale(scale) => {
                self.scale = scale;
                Task::none()
            }
            Message::Limit => self.ask(Command::Limits(Limits {
                fps: Some(self.fps),
                scale: Some(self.scale),
            })),
            Message::Answered(Ok(status)) => {
                self.status(status);
                Task::none()
            }
            Message::Applied(key, properties, Ok(status)) => {
                if properties.is_empty() {
                    self.config.wallpaper_properties.remove(&key);
                } else {
                    self.config.wallpaper_properties.insert(key, properties);
                }
                self.notice = config::save(&self.config).err();
                self.status(status);
                Task::none()
            }
            Message::Applied(_, _, Err(message)) => {
                self.notice = Some(message);
                Task::none()
            }
            Message::Answered(Err(message)) => {
                self.remember_error(&message);
                self.notice = Some(message);
                Task::none()
            }
            Message::Report => {
                let errors: Vec<String> = self.errors.iter().cloned().collect();
                let saved = self.picked().map(|item| {
                    report::Report {
                        item,
                        daemon: self.daemon.as_ref(),
                        errors: &errors,
                    }
                    .save()
                });
                match saved {
                    Some(Ok(path)) => {
                        self.notice = Some(format!(
                            "Saved a report to {}: attach it where you report the problem. Your home folder and user name are taken out.",
                            config::tilde(&path)
                        ));
                        if let Some(folder) = path.parent() {
                            let folder = folder.to_string_lossy().into_owned();
                            self.open(&folder);
                        }
                    }
                    Some(Err(message)) => self.notice = Some(message),
                    None => {}
                }
                Task::none()
            }
            Message::Daemon(daemon::Update::Event(event)) => self.heard(event),
            Message::Daemon(daemon::Update::Gone) => {
                self.daemon = None;
                Task::none()
            }
            Message::Page(page) => {
                self.page = page;
                match page {
                    Page::Workshop => self.open_workshop(),
                    Page::Library => self.rescan(),
                    Page::Settings => Task::none(),
                }
            }
            Message::Shop(shop) => self.shop(shop),
            Message::Steam(act) => self.steam(act),
            Message::Folder(folder) => {
                self.folder = folder;
                Task::none()
            }
            Message::AddFolder => {
                let path = config::expand(&self.folder);
                if !path.is_dir() {
                    self.notice = Some(format!("{} is not a folder", config::tilde(&path)));
                    return Task::none();
                }
                if !self.folders().contains(&path) {
                    self.config.folders.push(path);
                }
                self.folder.clear();
                self.keep()
            }
            Message::RemoveFolder(path) => {
                self.config.folders.retain(|folder| *folder != path);
                self.keep()
            }
            Message::Start => {
                self.notice = daemon::start().err();
                Task::none()
            }
            Message::Rescan => self.rescan(),
            Message::Open(link) => {
                self.open(&link);
                Task::none()
            }
            Message::Dismiss => {
                self.notice = None;
                Task::none()
            }
        }
    }

    fn keep(&mut self) -> Task<Message> {
        self.notice = config::save(&self.config).err();
        self.rescan()
    }

    pub fn open(&mut self, link: &str) {
        self.notice = std::process::Command::new("xdg-open")
            .arg(link)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|err| format!("xdg-open {link}: {err}"))
            .err();
    }

    fn pick(&mut self, path: PathBuf) {
        self.look = self.saved_look(&path);
        self.selected = Some(path);
    }

    pub fn saved_look(&self, path: &Path) -> Look {
        self.config
            .looks
            .get(&path.to_string_lossy().into_owned())
            .copied()
            .unwrap_or_default()
    }

    fn keep_look(&mut self) {
        let Some(path) = self
            .selected
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
        else {
            return;
        };
        if self.look.is_plain() {
            self.config.looks.remove(&path);
        } else {
            self.config.looks.insert(path, self.look);
        }
        self.notice = config::save(&self.config).err();
    }

    /// Monitors where the picked wallpaper is on now and the look would show: the chosen one,
    /// or every one showing it.
    fn adjustable(&self) -> Vec<String> {
        let Some(path) = &self.selected else {
            return Vec::new();
        };
        let scene = self.picked().is_some_and(|item| item.what == What::Scene);
        self.daemon
            .iter()
            .flat_map(|daemon| &daemon.outputs)
            .filter(|monitor| {
                monitor.source.as_ref() == Some(path)
                    && self
                        .target
                        .as_ref()
                        .is_none_or(|target| target == &monitor.name)
                    && (!scene || monitor.native_scene)
            })
            .map(|monitor| monitor.name.clone())
            .collect()
    }

    fn adjust(&self, draft: bool) -> Task<Message> {
        Task::batch(self.adjustable().into_iter().map(|output| {
            let command = Command::Adjust(Adjust {
                output: Some(output),
                look: self.look,
                draft,
            });
            Task::perform(daemon::send(command), Message::Sent)
        }))
    }

    pub fn ask(&self, command: Command) -> Task<Message> {
        Task::perform(daemon::ask(command), Message::Answered)
    }

    fn pump(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while self.busy < BUSY
            && let Some(item) = self.queue.pop_front()
        {
            self.busy += 1;
            tasks.push(Task::perform(library::thumbnail(item), |(path, handle)| {
                Message::Thumb(path, handle)
            }));
        }
        Task::batch(tasks)
    }

    fn status(&mut self, event: Event) {
        if let Event::Status {
            outputs,
            fps,
            scale,
            power,
        } = event
        {
            if self.daemon.is_none() {
                self.fps = fps;
                self.scale = scale.unwrap_or(1.0);
            }
            if let Some(target) = &self.target
                && !outputs.iter().any(|monitor| &monitor.name == target)
            {
                self.target = None;
            }
            self.daemon = Some(Daemon {
                power,
                outputs,
                fps,
                scale,
            });
        }
    }

    fn heard(&mut self, event: Event) -> Task<Message> {
        match event {
            status @ Event::Status { .. } => {
                self.status(status);
                Task::none()
            }
            Event::Ready { output, source, .. } => {
                if let Some(daemon) = &mut self.daemon
                    && let Some(monitor) = daemon.outputs.iter_mut().find(|m| m.name == output)
                {
                    monitor.source = Some(source.clone());
                }
                // A video or a shader may have just got its first still.
                let fresh = self
                    .items
                    .iter()
                    .find(|item| item.path == source && item.what == What::Shader)
                    .cloned();
                match fresh {
                    Some(item) => {
                        self.queue.push_back(item);
                        self.pump()
                    }
                    None => Task::none(),
                }
            }
            Event::Error { message, .. } => {
                self.remember_error(&message);
                self.notice = Some(message);
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let daemon = daemon::watch().map(Message::Daemon);
        let preview_keys = if self.preview.is_some() {
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::Preview(preview::Action::Close)),
                _ => None,
            })
        } else {
            Subscription::none()
        };
        let daemon = Subscription::batch([daemon, preview_keys]);
        if self.browser.waiting.is_empty() {
            daemon
        } else {
            // Only while Steam is downloading something asked for here.
            let watch = iced::time::every(std::time::Duration::from_secs(3))
                .map(|_| Message::Shop(browse::Shop::Watch));
            Subscription::batch([daemon, watch])
        }
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    pub fn visible(&self) -> Vec<&Item> {
        let search = self.search.trim().to_lowercase();
        self.items
            .iter()
            .filter(|item| match self.filter {
                Filter::All => true,
                Filter::Favorites => self.config.favorites.contains(&item.path),
                Filter::Pictures => item.what == What::Picture,
                Filter::Videos => item.what == What::Video,
                Filter::Shaders => item.what == What::Shader,
                Filter::Engine => item.engine,
            })
            .filter(|item| search.is_empty() || item.name.to_lowercase().contains(&search))
            .collect()
    }

    pub fn picked(&self) -> Option<&Item> {
        let selected = self.selected.as_ref()?;
        self.items.iter().find(|item| &item.path == selected)
    }

    fn remember_error(&mut self, message: &str) {
        if self.errors.len() == 20 {
            self.errors.pop_front();
        }
        self.errors.push_back(message.chars().take(500).collect());
    }

    /// The library's name for a file, which for Wallpaper Engine is its title.
    pub fn title(&self, path: &Path) -> String {
        self.items
            .iter()
            .find(|item| item.path == path)
            .map_or_else(|| engine::title(path), |item| item.name.clone())
    }

    /// Monitors showing this file right now.
    pub fn shown_on(&self, path: &Path) -> Vec<&str> {
        self.daemon
            .iter()
            .flat_map(|daemon| &daemon.outputs)
            .filter(|monitor| monitor.source.as_deref() == Some(path))
            .map(|monitor| monitor.name.as_str())
            .collect()
    }
}
