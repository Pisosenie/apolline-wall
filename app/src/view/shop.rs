use iced::widget::{
    Column, Row, button, column, container, image, responsive, row, scrollable, space, stack, text,
};
use iced::{Alignment, ContentFit, Element, Fill};

use super::{GAP, INSPECTOR, PREVIEW, TILE, monitors, segmented, steam};
use crate::browse::Shop;
use crate::download::{Act, Wanted};
use crate::library::What;
use crate::workshop::{self, Content, Entry, Scope, Sort, Type};
use crate::{App, Message, Page, steamcmd, style};

pub fn page(app: &App) -> Element<'_, Message> {
    let browser = &app.browser;
    let count = text(match (browser.loading, browser.entries.len()) {
        (true, 0) => "Loading the Workshop…".to_owned(),
        (_, n) => format!("{n} items"),
    })
    .size(13)
    .color(style::FAINT);
    let kinds = [
        (Type::Videos, "Videos"),
        (Type::Scenes, "Scenes"),
        (Type::Everything, "Everything"),
    ];
    let filters = match browser.query.scope {
        Scope::Workshop => row![
            segmented(
                [
                    (Type::Videos, "Videos"),
                    (Type::Scenes, "Scenes"),
                    (Type::Everything, "Everything"),
                    (Type::Collections, "Collections"),
                ],
                browser.query.kind,
                |kind| Message::Shop(Shop::Type(kind)),
            ),
            segmented(
                [
                    (Sort::Trending, "Trending"),
                    (Sort::Newest, "Newest"),
                    (Sort::Popular, "Most subscribed"),
                ],
                browser.query.sort,
                |sort| Message::Shop(Shop::Sort(sort)),
            ),
            space::horizontal(),
            count,
        ],
        _ => row![
            button(text("‹ Back").size(13))
                .padding([6, 12])
                .style(style::secondary)
                .on_press(Message::Shop(Shop::Back)),
            text(browser.heading.as_str())
                .size(16)
                .font(style::semibold())
                .wrapping(iced::widget::text::Wrapping::None),
            space::horizontal(),
            segmented(kinds, browser.query.kind, |kind| Message::Shop(Shop::Type(
                kind
            ))),
            count,
        ],
    }
    .spacing(12)
    .align_y(Alignment::Center);
    let body: Element<'_, Message> = match (&browser.error, browser.entries.is_empty()) {
        (Some(error), true) => notice(error, true),
        (None, true) if !browser.loading => {
            notice("Nothing matches. Try another kind or search.", false)
        }
        (None, true) => notice("Loading the Workshop…", false),
        _ => grid(app),
    };
    let mut listing = column![filters].spacing(14).width(Fill);
    if let Some(run) = steam::progress(app) {
        listing = listing.push(run);
    }
    row![listing.push(body), inspector(app)]
        .spacing(18)
        .height(Fill)
        .into()
}

fn notice(message: &str, retry: bool) -> Element<'_, Message> {
    let mut lines = column![text(message).size(15).color(style::SOFT)]
        .spacing(12)
        .align_x(Alignment::Center);
    if retry {
        lines = lines.push(
            button(text("Try again").size(13))
                .padding([6, 14])
                .style(style::secondary)
                .on_press(Message::Shop(Shop::Find)),
        );
    }
    container(lines).center(Fill).style(style::sunken).into()
}

fn grid(app: &App) -> Element<'_, Message> {
    let browser = &app.browser;
    responsive(move |size| {
        let columns = (((size.width + GAP) / (TILE + GAP)).floor() as usize).max(1);
        let mut rows = Column::new().spacing(GAP);
        for chunk in browser.entries.chunks(columns) {
            let mut line = Row::new().spacing(GAP);
            for entry in chunk {
                line = line.push(tile(app, entry));
            }
            for _ in chunk.len()..columns {
                line = line.push(space().width(Fill));
            }
            rows = rows.push(line);
        }
        if !browser.exhausted {
            let more = button(
                container(
                    text(if browser.loading {
                        "Loading…"
                    } else {
                        "More"
                    })
                    .size(13)
                    .font(style::medium()),
                )
                .center_x(Fill),
            )
            .padding([9, 16])
            .width(Fill)
            .style(style::secondary);
            rows = rows.push(if browser.loading {
                more
            } else {
                more.on_press(Message::Shop(Shop::More))
            });
        }
        if let Some(error) = &browser.error {
            rows = rows.push(text(error.as_str()).size(12).color(style::DANGER));
        }
        scrollable(container(rows).padding(iced::padding::right(10)))
            .direction(style::thin())
            .style(style::scroller)
            .height(Fill)
            .into()
    })
    .into()
}

fn tile<'a>(app: &'a App, entry: &'a Entry) -> Element<'a, Message> {
    let picked = app.browser.selected.as_ref() == Some(&entry.id);
    let mut layers = stack![preview(app, entry, PREVIEW)];
    if app.in_library(&entry.id) {
        layers = layers.push(
            container(
                container(text("In your library").size(11).font(style::medium()))
                    .padding([3, 8])
                    .style(style::pill(style::ACCENT)),
            )
            .padding(8)
            .align_bottom(Fill),
        );
    }
    let caption = row![
        container(
            text(entry.title.as_str())
                .size(13)
                .font(style::medium())
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Fill)
        .clip(true),
        text(match entry.content {
            Content::Collection(items) => format!("{items} items"),
            Content::Wallpaper(_) => workshop::count(entry.subscriptions),
        })
        .size(11)
        .color(style::FAINT),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    button(column![layers, caption].spacing(9))
        .padding(6)
        .width(Fill)
        .style(style::tile(picked))
        .on_press(Message::Shop(Shop::Select(entry.id.clone())))
        .into()
}

fn preview<'a>(app: &'a App, entry: &'a Entry, height: f32) -> Element<'a, Message> {
    match app.browser.thumbs.get(&entry.id) {
        Some(handle) => image(handle.clone())
            .width(Fill)
            .height(height)
            .content_fit(ContentFit::Cover)
            .border_radius(10.0)
            .into(),
        None => container(text(entry.content.label()).size(12).color(style::FAINT))
            .center_x(Fill)
            .center_y(height)
            .style(style::sunken)
            .into(),
    }
}

fn inspector(app: &App) -> Element<'_, Message> {
    let Some(entry) = app.browser.picked() else {
        return container(text("Pick a wallpaper").color(style::FAINT))
            .center_x(INSPECTOR)
            .center_y(Fill)
            .style(style::panel)
            .into();
    };
    let mut facts = vec![match entry.content {
        Content::Wallpaper(what) => what.label().to_owned(),
        Content::Collection(items) => format!("Collection of {items}"),
    }];
    facts.extend(entry.tags.iter().cloned());
    let mut numbers = Vec::new();
    if let Content::Wallpaper(_) = entry.content {
        numbers.push(format!(
            "{} subscribers",
            workshop::count(entry.subscriptions)
        ));
    }
    if let Content::Wallpaper(_) = entry.content {
        numbers.extend(Some(workshop::size(entry.size)).filter(|size| !size.is_empty()));
    }
    if entry.updated > 0 {
        numbers.push(format!("updated {}", workshop::date(entry.updated)));
    }
    let mut links = Row::new().spacing(6).push(small(
        "Open in the browser",
        Message::Open(workshop::web_page(&entry.id)),
    ));
    let author = Scope::Author(entry.creator.clone());
    if !entry.creator.is_empty() && app.browser.query.scope != author {
        links = links.push(small(
            "More from the author",
            Message::Shop(Shop::Enter(author, "Their wallpapers".into())),
        ));
    }
    let mut details = column![
        preview(app, entry, 186.0),
        column![
            text(entry.title.as_str()).size(20).font(style::semibold()),
            text(facts.join(" · ")).size(12).color(style::FAINT),
            text(numbers.join(" · ")).size(12).color(style::FAINT),
        ]
        .spacing(4),
        links,
    ]
    .spacing(16);
    let ready = app.downloaded(&entry.id);
    if ready.is_some() {
        details = details.push(super::section("Show on", monitors(app)));
    }
    details = details.push(space::vertical());
    let (note, label, action) = match entry.content {
        Content::Collection(_) => (
            None,
            "Open the collection",
            Some(Message::Shop(Shop::Enter(
                Scope::Collection(entry.id.clone()),
                entry.title.clone(),
            ))),
        ),
        Content::Wallpaper(what) => offer(app, entry, what),
    };
    if let Some(note) = note {
        details = details.push(text(note).size(12).color(style::SOFT));
    }
    if ready.is_some() && app.browser.updates.contains(&entry.id) && !app.steam.wants(&entry.id) {
        details = details.push(
            row![
                text("A newer version is out.")
                    .size(12)
                    .color(style::SOFT)
                    .width(Fill),
                small("Update", download(entry)),
            ]
            .align_y(Alignment::Center),
        );
    }
    let mut button = button(container(text(label).size(14).font(style::medium())).center_x(Fill))
        .padding([11, 16])
        .width(Fill)
        .style(style::primary);
    if let Some(message) = action
        && (ready.is_none() || app.daemon.is_some())
    {
        button = button.on_press(message);
    }
    details = details.push(button);
    if let Some(notice) = &app.notice {
        details = details.push(super::notice_line(notice));
    }
    container(details)
        .padding(16)
        .width(INSPECTOR)
        .height(Fill)
        .style(style::panel)
        .into()
}

/// A note on how to get it, the main button's label, and what the button does if it can.
type Offer = (Option<&'static str>, &'static str, Option<Message>);

/// How to get a wallpaper onto this machine: through the Steam client if there is one,
/// otherwise through steamcmd.
fn offer(app: &App, entry: &Entry, what: What) -> Offer {
    let id = &entry.id;
    if app.downloaded(id).is_some() {
        return (
            Some("It's in your library."),
            "Set as wallpaper",
            Some(Message::Shop(Shop::Set(id.clone()))),
        );
    }
    if let Some(why) = what.missing() {
        return (Some(why), "Not available yet", None);
    }
    if app.browser.waiting.contains(id) {
        return (
            Some(
                "Waiting for Steam to download it. It shows up here and in your library as soon as it's done.",
            ),
            "Downloading in Steam…",
            None,
        );
    }
    if workshop::steam_installed() {
        return (
            Some(
                "Steam downloads it when you subscribe, into the folder this app reads. It needs Wallpaper Engine in your Steam library, installed with Proton.",
            ),
            "Subscribe in Steam",
            Some(Message::Shop(Shop::Subscribe(id.clone()))),
        );
    }
    if !steamcmd::installed() {
        return (
            Some(
                "Getting it takes the Steam client with Wallpaper Engine, or steamcmd and a Steam account that owns Wallpaper Engine; Settings says how to get steamcmd.",
            ),
            "Not available yet",
            None,
        );
    }
    if app.steam.wants(id) {
        return (
            Some("steamcmd is on it. It shows up here and in your library when it's done."),
            "Downloading…",
            None,
        );
    }
    if app.config.steam_account.is_none() {
        return (
            Some(
                "steamcmd downloads it with a Steam account that owns Wallpaper Engine. Sign in with yours in Settings first.",
            ),
            "Sign in to Steam",
            Some(Message::Page(Page::Settings)),
        );
    }
    (
        Some("steamcmd downloads it with your Steam account into a folder this app reads."),
        "Download",
        Some(download(entry)),
    )
}

fn download(entry: &Entry) -> Message {
    Message::Steam(Act::Download(Wanted {
        id: entry.id.clone(),
        title: entry.title.clone(),
        size: entry.size,
    }))
}

fn small(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label).size(12))
        .padding([5, 10])
        .style(style::secondary)
        .on_press(message)
        .into()
}
