//! Steam's Web API, which needs no key for this: the details of items and collections, and
//! the community pages that list them.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::Deserialize;

use super::{
    APP, Content, Entry, PAGE, Page, Query, Scope, Type, author_name, author_url, ids, page_url,
};
use crate::library::What;

const DETAILS: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const COLLECTIONS: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
/// How many items one details request asks about.
const BATCH: usize = 100;
/// Collections carry no rating of their own, so each is judged by this many of its first
/// wallpapers.
const SAMPLE: usize = 4;

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
    creator: String,
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    tag: String,
}

#[derive(Deserialize)]
struct CollectionEnvelope {
    response: CollectionList,
}

#[derive(Deserialize)]
struct CollectionList {
    #[serde(default)]
    collectiondetails: Vec<Collection>,
}

#[derive(Deserialize)]
struct Collection {
    publishedfileid: String,
    result: i64,
    #[serde(default)]
    children: Vec<Child>,
}

#[derive(Deserialize)]
struct Child {
    publishedfileid: String,
    #[serde(default)]
    filetype: i64,
}

/// The items each collection holds, in its order, leaving out collections inside it.
pub fn children(json: &str) -> Result<HashMap<String, Vec<String>>, String> {
    let envelope: CollectionEnvelope =
        serde_json::from_str(json).map_err(|err| format!("Steam answered oddly: {err}"))?;
    Ok(envelope
        .response
        .collectiondetails
        .into_iter()
        .filter(|collection| collection.result == 1 && !collection.children.is_empty())
        .map(|collection| {
            let items = collection
                .children
                .into_iter()
                .filter(|child| child.filetype == 0)
                .map(|child| child.publishedfileid)
                .collect();
            (collection.publishedfileid, items)
        })
        .collect())
}

/// Wallpaper Engine items from a details answer, without anything rated Mature, and
/// without Questionable ones unless asked for. An item in `collections` is a collection
/// holding that many wallpapers.
pub fn entries(
    json: &str,
    questionable: bool,
    collections: &HashMap<String, usize>,
) -> Result<Vec<Entry>, String> {
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
            });
            let content = match (what, collections.get(&detail.publishedfileid)) {
                (_, Some(&count)) => Content::Collection(count),
                (Some(what), None) => Content::Wallpaper(what),
                (None, None) => return None,
            };
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
                content,
                creator: detail.creator,
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

pub(super) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent("apolline-wallpapers")
        .build()
        .into()
}

fn page_of(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    agent
        .get(url)
        .call()
        .and_then(|mut answer| answer.body_mut().read_to_string())
        .map_err(|err| format!("The Workshop didn't load: {err}"))
}

/// Posts `ids` as Steam's `publishedfileids[n]` form fields.
fn ask(agent: &ureq::Agent, url: &str, count: &str, ids: &[String]) -> Result<String, String> {
    let mut form = vec![(count.to_owned(), ids.len().to_string())];
    form.extend(
        ids.iter()
            .enumerate()
            .map(|(at, id)| (format!("publishedfileids[{at}]"), id.clone())),
    );
    agent
        .post(url)
        .send_form(form)
        .and_then(|mut answer| answer.body_mut().read_to_string())
        .map_err(|err| format!("The Workshop's details didn't load: {err}"))
}

/// Details for any number of items, in the order of `ids`.
fn details(
    agent: &ureq::Agent,
    ids: &[String],
    questionable: bool,
    collections: &HashMap<String, usize>,
) -> Result<Vec<Entry>, String> {
    let mut found = Vec::new();
    for batch in ids.chunks(BATCH) {
        let json = ask(agent, DETAILS, "itemcount", batch)?;
        found.extend(entries(&json, questionable, collections)?);
    }
    Ok(ids
        .iter()
        .filter_map(|id| found.iter().find(|entry| &entry.id == id).cloned())
        .collect())
}

fn collections(
    agent: &ureq::Agent,
    ids: &[String],
) -> Result<HashMap<String, Vec<String>>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    children(&ask(agent, COLLECTIONS, "collectioncount", ids)?)
}

/// How many wallpapers each collection holds, leaving out empty ones and any where a sampled
/// wallpaper is rated past what is shown.
fn rated_collections(
    agent: &ureq::Agent,
    held: HashMap<String, Vec<String>>,
    questionable: bool,
) -> Result<HashMap<String, usize>, String> {
    let sample: Vec<String> = held
        .values()
        .flat_map(|items| items.iter().take(SAMPLE).cloned())
        .collect();
    let mut flagged = HashSet::new();
    for batch in sample.chunks(BATCH) {
        flagged.extend(too_rated(
            &ask(agent, DETAILS, "itemcount", batch)?,
            questionable,
        )?);
    }
    Ok(held
        .into_iter()
        .filter(|(_, items)| {
            !items.is_empty() && items.iter().take(SAMPLE).all(|id| !flagged.contains(id))
        })
        .map(|(id, items)| (id, items.len()))
        .collect())
}

/// The items in a details answer rated past what is shown.
pub fn too_rated(json: &str, questionable: bool) -> Result<HashSet<String>, String> {
    let envelope: Envelope =
        serde_json::from_str(json).map_err(|err| format!("Steam answered oddly: {err}"))?;
    Ok(envelope
        .response
        .publishedfiledetails
        .into_iter()
        .filter(|detail| {
            let rated = |rating: &str| detail.tags.iter().any(|tag| tag.tag == rating);
            rated("Mature") || (!questionable && rated("Questionable"))
        })
        .map(|detail| detail.publishedfileid)
        .collect())
}

pub async fn browse(query: Query, page: u32) -> Result<Page, String> {
    tokio::task::spawn_blocking(move || {
        let agent = agent();
        let none = HashMap::new();
        let page = match &query.scope {
            Scope::Workshop => {
                let ids = ids(&page_of(&agent, &page_url(&query, page))?);
                let counts = match query.kind {
                    Type::Collections => {
                        rated_collections(&agent, collections(&agent, &ids)?, query.questionable)?
                    }
                    _ => none,
                };
                Page {
                    entries: details(&agent, &ids, query.questionable, &counts)?,
                    author: None,
                    last: ids.len() < PAGE / 2,
                }
            }
            Scope::Author(steamid) => {
                let html = page_of(&agent, &author_url(steamid, page))?;
                let ids = ids(&html);
                Page {
                    entries: details(&agent, &ids, query.questionable, &none)?,
                    author: author_name(&html),
                    last: ids.len() < PAGE,
                }
            }
            Scope::Collection(id) => {
                let items = collections(&agent, std::slice::from_ref(id))?
                    .remove(id)
                    .unwrap_or_default();
                Page {
                    entries: details(&agent, &items, query.questionable, &none)?,
                    author: None,
                    last: true,
                }
            }
        };
        Ok(Page {
            entries: page
                .entries
                .into_iter()
                .filter(|entry| query.kind.fits(entry))
                .collect(),
            ..page
        })
    })
    .await
    .map_err(|err| err.to_string())?
}

/// Which of the items downloaded here, by id and update time, have a newer version out.
pub async fn updates(local: HashMap<String, u64>) -> Result<HashSet<String>, String> {
    tokio::task::spawn_blocking(move || {
        let ids: Vec<String> = local.keys().cloned().collect();
        let found = details(&agent(), &ids, true, &HashMap::new())?;
        Ok(found
            .into_iter()
            .filter(|entry| {
                local
                    .get(&entry.id)
                    .is_some_and(|&time| entry.updated > time)
            })
            .map(|entry| entry.id)
            .collect())
    })
    .await
    .map_err(|err| err.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let none = HashMap::new();
        let shown = entries(json, false, &none).unwrap();
        assert_eq!(shown.len(), 1);
        let leopard = &shown[0];
        assert_eq!(
            (leopard.title.as_str(), leopard.content, leopard.size),
            ("Snow leopard", Content::Wallpaper(What::Video), 9_370_203)
        );
        assert_eq!(leopard.tags, ["Animal", "1920 x 1080"]);
        let ids: Vec<String> = entries(json, true, &none)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["1", "2"]);
        assert!(
            entries("<html>", false, &none)
                .unwrap_err()
                .contains("oddly")
        );
    }

    #[test]
    fn collections_hold_their_wallpapers_but_not_other_collections() {
        let json = r#"{"response":{"result":1,"resultcount":2,"collectiondetails":[
            {"publishedfileid":"9","result":1,"children":[
                {"publishedfileid":"3","sortorder":0,"filetype":2},
                {"publishedfileid":"1","sortorder":0,"filetype":0},
                {"publishedfileid":"2","sortorder":1,"filetype":0}]},
            {"publishedfileid":"8","result":9}]}}"#;
        let held = children(json).unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held["9"], ["1", "2"]);
        let details = r#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"9","result":1,"title":"Calm","creator":"76561197991312497",
             "consumer_app_id":431960,"tags":[{"tag":"Relaxing"}]}]}}"#;
        let counts = HashMap::from([("9".to_owned(), 2)]);
        let found = entries(details, false, &counts).unwrap();
        assert_eq!(found[0].content, Content::Collection(2));
        assert_eq!(found[0].creator, "76561197991312497");
        assert!(entries(details, false, &HashMap::new()).unwrap().is_empty());
    }

    #[test]
    fn a_collection_is_judged_by_the_ratings_of_what_it_holds() {
        let json = r#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"1","result":1,"tags":[{"tag":"Video"},{"tag":"Everyone"}]},
            {"publishedfileid":"2","result":1,"tags":[{"tag":"Video"},{"tag":"Questionable"}]},
            {"publishedfileid":"3","result":1,"tags":[{"tag":"Scene"},{"tag":"Mature"}]}]}}"#;
        let flagged = |questionable| {
            let mut ids: Vec<String> = too_rated(json, questionable).unwrap().into_iter().collect();
            ids.sort();
            ids
        };
        assert_eq!(flagged(false), ["2", "3"]);
        assert_eq!(flagged(true), ["3"]);
    }
}
