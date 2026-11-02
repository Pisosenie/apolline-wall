use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use iced::widget::image::Handle;

use crate::config;
use crate::library::What;

mod api;

pub use api::{browse, updates};

const APP: &str = "431960";
const PAGE: usize = 30;
/// Workshop previews kept on disk; the oldest go first.
const PREVIEW_CACHE: u64 = 150 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Trending,
    Newest,
    Popular,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Videos,
    Scenes,
    Everything,
    Collections,
}

impl Type {
    fn fits(self, entry: &Entry) -> bool {
        match self {
            Type::Videos => entry.content == Content::Wallpaper(What::Video),
            Type::Scenes => entry.content == Content::Wallpaper(What::Scene),
            Type::Everything => true,
            Type::Collections => matches!(entry.content, Content::Collection(_)),
        }
    }
}

/// Where the list comes from: the whole Workshop, one collection, or one author's items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Workshop,
    Collection(String),
    Author(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub sort: Sort,
    pub kind: Type,
    pub search: String,
    pub questionable: bool,
    pub scope: Scope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Wallpaper(What),
    /// A collection and how many wallpapers it holds.
    Collection(usize),
}

impl Content {
    pub fn label(self) -> &'static str {
        match self {
            Content::Wallpaper(what) => what.label(),
            Content::Collection(_) => "Collection",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub preview: Option<String>,
    pub content: Content,
    /// The author's SteamID64.
    pub creator: String,
    pub size: u64,
    pub subscriptions: u64,
    pub updated: u64,
    /// Genre and resolution, the tags worth reading.
    pub tags: Vec<String>,
}

/// One answer to a listing: its entries, the author's name on an author's page, and whether
/// nothing comes after it.
#[derive(Clone, Debug)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub author: Option<String>,
    pub last: bool,
}

/// The list to go back to from a collection or an author.
pub struct Back {
    pub query: Query,
    pub selected: Option<String>,
}

/// What the Workshop tab shows: the pages loaded so far, their previews, and the items
/// handed to Steam to download.
pub struct Browser {
    pub query: Query,
    pub entries: Vec<Entry>,
    pub pages: u32,
    pub loading: bool,
    pub exhausted: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub thumbs: HashMap<String, Handle>,
    pub queue: VecDeque<Entry>,
    pub busy: usize,
    pub waiting: HashSet<String>,
    /// The collection's title or the author's name while one is open.
    pub heading: String,
    pub back: Option<Back>,
    /// Items downloaded here that the Workshop has a newer version of.
    pub updates: HashSet<String>,
    pub checked: bool,
}

impl Browser {
    pub fn new(questionable: bool) -> Self {
        Browser {
            query: Query {
                sort: Sort::Trending,
                kind: Type::Videos,
                search: String::new(),
                questionable,
                scope: Scope::Workshop,
            },
            entries: Vec::new(),
            pages: 0,
            loading: false,
            exhausted: false,
            error: None,
            selected: None,
            thumbs: HashMap::new(),
            queue: VecDeque::new(),
            busy: 0,
            waiting: HashSet::new(),
            heading: String::new(),
            back: None,
            updates: HashSet::new(),
            checked: false,
        }
    }

    pub fn restart(&mut self) {
        self.entries.clear();
        self.pages = 0;
        self.exhausted = false;
        self.error = None;
        self.queue.clear();
    }

    /// Adds a loaded page, skipping items already shown, and queues their previews.
    pub fn add(&mut self, page: Page) {
        self.pages += 1;
        self.exhausted = page.last;
        if let Some(author) = page.author {
            self.heading = author;
        }
        for entry in page.entries {
            if !self.entries.iter().any(|known| known.id == entry.id) {
                if !self.thumbs.contains_key(&entry.id) {
                    self.queue.push_back(entry.clone());
                }
                self.entries.push(entry);
            }
        }
        let shown = |id: &String| self.entries.iter().any(|entry| &entry.id == id);
        if !self.selected.as_ref().is_some_and(shown) {
            self.selected = self.entries.first().map(|entry| entry.id.clone());
        }
    }

    pub fn picked(&self) -> Option<&Entry> {
        let id = self.selected.as_ref()?;
        self.entries.iter().find(|entry| &entry.id == id)
    }
}

pub fn page_url(query: &Query, page: u32) -> String {
    let search = query.search.trim();
    let sort = if search.is_empty() {
        match query.sort {
            Sort::Trending => "trend",
            Sort::Newest => "mostrecent",
            Sort::Popular => "totaluniquesubscribers",
        }
    } else {
        "textsearch"
    };
    let section = match query.kind {
        Type::Collections => "collections",
        _ => "readytouseitems",
    };
    let mut url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={APP}&section={section}&numperpage={PAGE}&p={page}&browsesort={sort}&actualsort={sort}"
    );
    if sort == "trend" {
        url.push_str("&days=7");
    }
    let tag = match query.kind {
        Type::Videos => Some("Video"),
        Type::Scenes => Some("Scene"),
        Type::Everything | Type::Collections => None,
    };
    // Collections carry no rating; what they hold is filtered when one is opened.
    let rated = !query.questionable && query.kind != Type::Collections;
    for tag in tag.into_iter().chain(rated.then_some("Everyone")) {
        url.push_str("&requiredtags%5B%5D=");
        url.push_str(tag);
    }
    if !search.is_empty() {
        url.push_str("&searchtext=");
        url.push_str(&encode(search));
    }
    url
}

pub fn author_url(steamid: &str, page: u32) -> String {
    format!(
        "https://steamcommunity.com/profiles/{steamid}/myworkshopfiles/?appid={APP}&numperpage={PAGE}&p={page}"
    )
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            b' ' => "+".to_owned(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Item ids in a browse page, in order. Reading only the links keeps working when Steam
/// changes the rest of its markup.
pub fn ids(html: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for piece in html.split("filedetails/?id=").skip(1) {
        let id: String = piece.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() && !found.contains(&id) {
            found.push(id);
        }
    }
    found
}

/// The name in an author's page title, `Steam Community :: name :: Workshop Items`.
pub fn author_name(html: &str) -> Option<String> {
    let title = html.split_once("<title>")?.1.split_once("</title>")?.0;
    let parts: Vec<&str> = title.split(" :: ").collect();
    let [_, name @ .., last] = parts.as_slice() else {
        return None;
    };
    if name.is_empty() || !last.contains("Workshop") {
        return None;
    }
    let name = name.join(" :: ");
    let name = name.trim();
    let name = name
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    (!name.is_empty()).then_some(name)
}

/// Drops the oldest Workshop previews once they take more than their share of the disk.
pub async fn prune_previews() {
    let _ =
        tokio::task::spawn_blocking(|| prune(&config::cache_dir().join("workshop"), PREVIEW_CACHE))
            .await;
}

fn prune(folder: &Path, limit: u64) {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), entry.path()))
        })
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    let mut kept = 0;
    for (_, size, path) in files {
        kept += size;
        if kept > limit {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The item's Workshop preview, shrunk and cached by id and update time.
pub async fn preview(entry: Entry) -> (String, Option<Handle>) {
    let id = entry.id.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let cache = config::cache_dir()
            .join("workshop")
            .join(format!("{}-{}.png", entry.id, entry.updated));
        if !cache.exists() {
            let bytes = api::agent()
                .get(entry.preview.as_deref()?)
                .call()
                .ok()?
                .body_mut()
                .with_config()
                .limit(32 << 20)
                .read_to_vec()
                .ok()?;
            let image = crate::library::still(&bytes)?;
            std::fs::create_dir_all(cache.parent()?).ok()?;
            crate::library::save_thumb(&image, &cache).then_some(())?;
        }
        Some(Handle::from_path(cache))
    })
    .await
    .ok()
    .flatten();
    (id, handle)
}

pub fn steam_page(id: &str) -> String {
    format!("steam://url/CommunityFilePage/{id}")
}

pub fn web_page(id: &str) -> String {
    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}")
}

/// Whether there is a Steam client to hand subscriptions to, native or Flatpak.
pub fn steam_installed() -> bool {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("steam").is_file()));
    on_path
        || config::home()
            .join(".var/app/com.valvesoftware.Steam")
            .is_dir()
}

/// Megabytes or gigabytes, the way Steam writes sizes.
pub fn size(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b => format!("{} MB", (b as f64 / (1u64 << 20) as f64).ceil()),
    }
}

pub fn count(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}k", n / 1000),
        n if n >= 1000 => format!("{:.1}k", n as f64 / 1000.0),
        n => n.to_string(),
    }
}

/// A Unix time as a calendar date, without a date library.
pub fn date(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(sort: Sort, kind: Type, search: &str, questionable: bool) -> Query {
        Query {
            sort,
            kind,
            search: search.into(),
            questionable,
            scope: Scope::Workshop,
        }
    }

    #[test]
    fn the_browse_url_carries_kind_sort_rating_and_search() {
        let url = page_url(&query(Sort::Trending, Type::Videos, "", false), 2);
        assert!(
            url.contains("p=2&browsesort=trend&actualsort=trend&days=7"),
            "{url}"
        );
        assert!(
            url.ends_with("&requiredtags%5B%5D=Video&requiredtags%5B%5D=Everyone"),
            "{url}"
        );
        let url = page_url(
            &query(Sort::Popular, Type::Everything, "neon city", true),
            1,
        );
        assert!(url.contains("browsesort=textsearch"), "{url}");
        assert!(url.ends_with("&searchtext=neon+city"), "{url}");
        assert!(!url.contains("requiredtags"), "{url}");
        let url = page_url(&query(Sort::Newest, Type::Collections, "", false), 1);
        assert!(url.contains("section=collections"), "{url}");
        assert!(!url.contains("requiredtags"), "{url}");
        assert_eq!(encode("ночь & rain"), "%D0%BD%D0%BE%D1%87%D1%8C+%26+rain");
    }

    #[test]
    fn ids_come_from_links_once_each_in_order() {
        let html = r#"<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=38&searchtext=">
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=12"><img></a>
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=38">again</a>"#;
        assert_eq!(ids(html), ["38", "12"]);
    }

    #[test]
    fn an_authors_name_comes_from_the_page_title() {
        let html = "<head><title>Steam Community :: Tom &amp; Jerry :: Workshop Items</title>";
        assert_eq!(author_name(html).as_deref(), Some("Tom & Jerry"));
        assert_eq!(author_name("<title>Steam Community :: Error</title>"), None);
    }

    #[test]
    fn the_preview_cache_keeps_the_newest_within_its_limit() {
        let folder = std::env::temp_dir().join(format!("apolline-prune-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        for (name, age) in [("old.png", 300), ("mid.png", 200), ("new.png", 100)] {
            let path = folder.join(name);
            std::fs::write(&path, [0u8; 10]).unwrap();
            let time = SystemTime::now() - std::time::Duration::from_secs(age);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        prune(&folder, 25);
        let mut left: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["mid.png", "new.png"]);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn numbers_read_like_steam_writes_them() {
        assert_eq!(size(9_370_203), "9 MB");
        assert_eq!(size(3 << 30), "3.0 GB");
        assert_eq!(count(9500), "9.5k");
        assert_eq!(count(42_000), "42k");
        assert_eq!(count(812), "812");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_793_296_108), "2026-10-29");
    }
}
