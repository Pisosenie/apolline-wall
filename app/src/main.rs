mod browse;
mod config;
mod daemon;
mod engine;
mod library;
mod style;
mod view;
mod workshop;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use apolline_wall_protocol::protocol::{Command, Event, Fit, Limits, Monitor, Pause, Show};
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
    Pictures,
    Videos,
    Shaders,
    Engine,
}

pub struct Daemon {
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
    pub folder: String,
    pub fps: u32,
    pub scale: f32,
    pub browser: workshop::Browser,
}

#[derive(Clone, Debug)]
pub enum Message {
    Scanned(Vec<Item>),
    Thumb(PathBuf, Option<Handle>),
    Select(PathBuf),
    Filter(Filter),
    Search(String),
    Target(Option<String>),
    Fit(Fit),
    Apply,
    Pause(Option<String>, bool),
    Daemon(daemon::Update),
    Answered(Result<Event, String>),
    Page(Page),
    Folder(String),
    AddFolder,
    RemoveFolder(PathBuf),
    Fps(u32),
    Scale(f32),
    Limit,
    Start,
    Rescan,
    Dismiss,
    Open(String),
    Shop(browse::Shop),
}

impl App {
    fn new(page: Page) -> (Self, Task<Message>) {
        let config = config::load();
        let mut app = App {
            browser: workshop::Browser::new(config.questionable),
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
        config::defaults()
            .into_iter()
            .chain(self.config.folders.iter().cloned())
            .collect()
    }

    pub fn rescan(&self) -> Task<Message> {
        Task::perform(
            library::scan(self.folders(), engine::folders()),
            Message::Scanned,
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
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
                if gone {
                    let playable = items.iter().find(|item| item.what.missing().is_none());
                    self.selected = playable.or(items.first()).map(|item| item.path.clone());
                }
                self.items = items;
                self.pump()
            }
            Message::Thumb(path, handle) => {
                self.busy = self.busy.saturating_sub(1);
                if let Some(handle) = handle {
                    self.thumbs.insert(path, handle);
                }
                self.pump()
            }
            Message::Select(path) => {
                self.selected = Some(path);
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
                .map(|item| item.path.clone())
            {
                Some(source) => self.ask(Command::Show(Show {
                    output: self.target.clone(),
                    source,
                    fit: self.fit,
                    transition_ms: 600,
                })),
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
            Message::Answered(Err(message)) => {
                self.notice = Some(message);
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
                self.notice = Some(message);
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let daemon = daemon::watch().map(Message::Daemon);
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

    /// The library's name for a file, which for Wallpaper Engine is its title.
    pub fn title(&self, path: &Path) -> String {
        self.items
            .iter()
            .find(|item| item.path == path)
            .map_or_else(|| library::name_of(path), |item| item.name.clone())
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
