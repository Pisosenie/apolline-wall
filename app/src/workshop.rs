use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use iced::widget::image::Handle;
use serde::Deserialize;

use crate::config;
use crate::library::What;

const APP: &str = "431960";
const PAGE: usize = 30;
const DETAILS: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub sort: Sort,
    pub kind: Type,
    pub search: String,
    pub questionable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub preview: Option<String>,
    pub what: What,
    pub size: u64,
    pub subscriptions: u64,
    pub updated: u64,
    /// Genre and resolution, the tags worth reading.
    pub tags: Vec<String>,
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
}

impl Browser {
    pub fn new(questionable: bool) -> Self {
        Browser {
            query: Query {
                sort: Sort::Trending,
                kind: Type::Videos,
                search: String::new(),
                questionable,
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
    pub fn add(&mut self, entries: Vec<Entry>) {
        self.pages += 1;
        self.exhausted = entries.len() < PAGE / 2;
        for entry in entries {
            if !self.entries.iter().any(|known| known.id == entry.id) {
                if !self.thumbs.contains_key(&entry.id) {
                    self.queue.push_back(entry.clone());
                }
                self.entries.push(entry);
            }
        }
        if self.selected.is_none() {
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
    let mut url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={APP}&section=readytouseitems&numperpage={PAGE}&p={page}&browsesort={sort}&actualsort={sort}"
    );
    if sort == "trend" {
        url.push_str("&days=7");
    }
    let tag = match query.kind {
        Type::Videos => Some("Video"),
        Type::Scenes => Some("Scene"),
        Type::Everything => None,
    };
    for tag in tag
        .into_iter()
        .chain((!query.questionable).then_some("Everyone"))
    {
        url.push_str("&requiredtags%5B%5D=");
        url.push_str(tag);
    }
    if !search.is_empty() {
        url.push_str("&searchtext=");
        url.push_str(&encode(search));
    }
    url
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

#[derive(Deserialize)]
struct Envelope {
    response: Details,
}

#[derive(Deserialize)]
struct Details {
    #[serde(default)]
    publishedfiledetails: Vec<Detail>,
}

#[derive(Deserialize)]
struct Detail {
    publishedfileid: String,
    result: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    preview_url: String,
    #[serde(default)]
    file_size: serde_json::Value,
    #[serde(default)]
    time_updated: u64,
    #[serde(default)]
    subscriptions: u64,
    #[serde(default)]
    consumer_app_id: u64,
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    tag: String,
}

/// Wallpaper Engine items from a details answer, without anything rated Mature, and
/// without Questionable ones unless asked for.
pub fn entries(json: &str, questionable: bool) -> Result<Vec<Entry>, String> {
    let envelope: Envelope =
        serde_json::from_str(json).map_err(|err| format!("Steam answered oddly: {err}"))?;
    Ok(envelope
        .response
        .publishedfiledetails
        .into_iter()
        .filter(|detail| detail.result == 1 && detail.consumer_app_id.to_string() == APP)
        .filter_map(|detail| {
            let tags: Vec<String> = detail.tags.into_iter().map(|tag| tag.tag).collect();
            let rated = |rating: &str| tags.iter().any(|tag| tag == rating);
            if rated("Mature") || (rated("Questionable") && !questionable) {
                return None;
            }
            let what = tags.iter().find_map(|tag| match tag.as_str() {
                "Video" => Some(What::Video),
                "Scene" => Some(What::Scene),
                "Web" => Some(What::Web),
                "Application" => Some(What::Program),
                _ => None,
            })?;
            let readable = tags
                .iter()
                .filter(|tag| {
                    !matches!(
                        tag.as_str(),
                        "Video"
                            | "Scene"
                            | "Web"
                            | "Application"
                            | "Wallpaper"
                            | "Everyone"
                            | "Questionable"
                            | "Mature"
                            | "Preset"
                    )
                })
                .cloned()
                .collect();
            Some(Entry {
                title: detail.title.trim().to_owned(),
                preview: Some(detail.preview_url).filter(|url| url.starts_with("https://")),
                what,
                size: match &detail.file_size {
                    serde_json::Value::String(size) => size.parse().unwrap_or(0),
                    other => other.as_u64().unwrap_or(0),
                },
                subscriptions: detail.subscriptions,
                updated: detail.time_updated,
                tags: readable,
                id: detail.publishedfileid,
            })
        })
        .collect())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent("apolline-wallpapers")
        .build()
        .into()
}

pub async fn browse(query: Query, page: u32) -> Result<Vec<Entry>, String> {
    tokio::task::spawn_blocking(move || {
        let agent = agent();
        let html = agent
            .get(page_url(&query, page))
            .call()
            .and_then(|mut answer| answer.body_mut().read_to_string())
            .map_err(|err| format!("The Workshop didn't load: {err}"))?;
        let ids = ids(&html);
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut form = vec![("itemcount".to_owned(), ids.len().to_string())];
        form.extend(
            ids.iter()
                .enumerate()
                .map(|(at, id)| (format!("publishedfileids[{at}]"), id.clone())),
        );
        let json = agent
            .post(DETAILS)
            .send_form(form)
            .and_then(|mut answer| answer.body_mut().read_to_string())
            .map_err(|err| format!("The Workshop's details didn't load: {err}"))?;
        let found = entries(&json, query.questionable)?;
        // Keep the page's order, which is the order Steam sorted them in.
        Ok(ids
            .iter()
            .filter_map(|id| found.iter().find(|entry| &entry.id == id).cloned())
            .collect())
    })
    .await
    .map_err(|err| err.to_string())?
}

/// The item's Workshop preview, shrunk and cached by id and update time.
pub async fn preview(entry: Entry) -> (String, Option<Handle>) {
    let id = entry.id.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let cache = config::cache_dir()
            .join("workshop")
            .join(format!("{}-{}.png", entry.id, entry.updated));
        if !cache.exists() {
            let bytes = agent()
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
    fn details_become_entries_and_ratings_filter_them() {
        let json = r#"{"response":{"result":1,"resultcount":3,"publishedfiledetails":[
            {"publishedfileid":"1","result":1,"title":" Snow leopard ","preview_url":"https://images.example/1/",
             "file_size":"9370203","time_updated":1793296108,"subscriptions":3421,"consumer_app_id":431960,
             "tags":[{"tag":"Video"},{"tag":"Animal"},{"tag":"Wallpaper"},{"tag":"1920 x 1080"},{"tag":"Everyone"}]},
            {"publishedfileid":"2","result":1,"title":"Racing","file_size":"165969171","consumer_app_id":431960,
             "tags":[{"tag":"Video"},{"tag":"Questionable"}]},
            {"publishedfileid":"3","result":1,"title":"Night","consumer_app_id":431960,
             "tags":[{"tag":"Scene"},{"tag":"Mature"}]},
            {"publishedfileid":"4","result":9}]}}"#;
        let shown = entries(json, false).unwrap();
        assert_eq!(shown.len(), 1);
        let leopard = &shown[0];
        assert_eq!(
            (leopard.title.as_str(), leopard.what, leopard.size),
            ("Snow leopard", What::Video, 9_370_203)
        );
        assert_eq!(leopard.tags, ["Animal", "1920 x 1080"]);
        let ids: Vec<String> = entries(json, true)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["1", "2"]);
        assert!(entries("<html>", false).unwrap_err().contains("oddly"));
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
