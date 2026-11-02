use apolline_wall_protocol::protocol::{Command, Show};
use iced::Task;
use iced::widget::image::Handle;

use crate::workshop::{self, Entry, Sort, Type};
use crate::{App, Message, config, engine};

/// Workshop previews fetched at once.
const BUSY: usize = 6;

#[derive(Clone, Debug)]
pub enum Shop {
    Type(Type),
    Sort(Sort),
    Search(String),
    Find,
    More,
    Loaded(Result<Vec<Entry>, String>),
    Preview(String, Option<Handle>),
    Select(String),
    Subscribe(String),
    Set(String),
    Watch,
    Questionable(bool),
}

impl App {
    /// The first visit loads the first page; later visits keep what was there.
    pub fn open_workshop(&mut self) -> Task<Message> {
        if self.browser.pages == 0 && !self.browser.loading {
            self.load()
        } else {
            Task::none()
        }
    }

    fn load(&mut self) -> Task<Message> {
        self.browser.loading = true;
        self.browser.error = None;
        let page = self.browser.pages + 1;
        Task::perform(
            workshop::browse(self.browser.query.clone(), page),
            |result| Message::Shop(Shop::Loaded(result)),
        )
    }

    fn reload(&mut self) -> Task<Message> {
        self.browser.restart();
        self.load()
    }

    pub fn shop(&mut self, shop: Shop) -> Task<Message> {
        match shop {
            Shop::Type(kind) => {
                self.browser.query.kind = kind;
                self.reload()
            }
            Shop::Sort(sort) => {
                self.browser.query.sort = sort;
                self.reload()
            }
            Shop::Search(text) => {
                self.browser.query.search = text;
                Task::none()
            }
            Shop::Find => self.reload(),
            Shop::More if self.browser.loading || self.browser.exhausted => Task::none(),
            Shop::More => self.load(),
            Shop::Loaded(result) => {
                self.browser.loading = false;
                match result {
                    Ok(entries) => {
                        self.browser.add(entries);
                        self.previews()
                    }
                    Err(message) => {
                        self.browser.error = Some(message);
                        Task::none()
                    }
                }
            }
            Shop::Preview(id, handle) => {
                self.browser.busy = self.browser.busy.saturating_sub(1);
                if let Some(handle) = handle {
                    self.browser.thumbs.insert(id, handle);
                }
                self.previews()
            }
            Shop::Select(id) => {
                self.browser.selected = Some(id);
                Task::none()
            }
            Shop::Subscribe(id) => {
                if workshop::steam_installed() {
                    self.open(&workshop::steam_page(&id));
                    self.browser.waiting.insert(id);
                } else {
                    self.notice = Some(
                        "Steam isn't installed. Install it, then Wallpaper Engine in Steam with \
                         Proton: its Workshop downloads land where this app looks."
                            .into(),
                    );
                }
                Task::none()
            }
            Shop::Set(id) => match self.downloaded(&id) {
                Some(source) => self.ask(Command::Show(Show {
                    output: self.target.clone(),
                    source,
                    fit: self.fit,
                    transition_ms: 600,
                })),
                None => Task::none(),
            },
            Shop::Watch => {
                let arrived: Vec<String> = self
                    .browser
                    .waiting
                    .iter()
                    .filter(|id| engine::downloaded(id))
                    .cloned()
                    .collect();
                if arrived.is_empty() {
                    return Task::none();
                }
                for id in &arrived {
                    self.browser.waiting.remove(id);
                }
                self.rescan()
            }
            Shop::Questionable(on) => {
                self.config.questionable = on;
                self.browser.query.questionable = on;
                self.notice = config::save(&self.config).err();
                self.reload()
            }
        }
    }

    fn previews(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while self.browser.busy < BUSY
            && let Some(entry) = self.browser.queue.pop_front()
        {
            self.browser.busy += 1;
            tasks.push(Task::perform(workshop::preview(entry), |(id, handle)| {
                Message::Shop(Shop::Preview(id, handle))
            }));
        }
        Task::batch(tasks)
    }

    /// The file to show for a Workshop item already on this machine, if it can be shown.
    pub fn downloaded(&self, id: &str) -> Option<std::path::PathBuf> {
        self.items
            .iter()
            .find(|item| item.workshop.as_deref() == Some(id) && item.what.missing().is_none())
            .map(|item| item.path.clone())
    }

    pub fn in_library(&self, id: &str) -> bool {
        self.items
            .iter()
            .any(|item| item.workshop.as_deref() == Some(id))
    }
}
