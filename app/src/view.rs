mod look;
mod shop;

use apolline_wall_protocol::protocol::Fit;
use iced::widget::{
    Column, Row, button, column, container, image, responsive, row, scrollable, slider, space,
    stack, text, text_input, toggler,
};
use iced::{Alignment, ContentFit, Element, Fill, Length};

use crate::browse::Shop;
use crate::library::{Item, What};
use crate::{App, Filter, Message, Page, config, style, workshop};

const TILE: f32 = 214.0;
const GAP: f32 = 14.0;
const PREVIEW: f32 = 124.0;
const INSPECTOR: f32 = 330.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let page = match app.page {
        Page::Library => library(app),
        Page::Workshop => shop::page(app),
        Page::Settings => settings(app),
    };
    container(column![top(app), page, playing(app)].spacing(16))
        .padding(20)
        .style(style::window)
        .width(Fill)
        .height(Fill)
        .into()
}

fn top(app: &App) -> Element<'_, Message> {
    let mut bar = row![
        text("Wallpapers").size(22).font(style::semibold()),
        space().width(18),
        segmented(
            [
                (Page::Library, "Library"),
                (Page::Workshop, "Steam Workshop"),
                (Page::Settings, "Settings"),
            ],
            app.page,
            Message::Page,
        ),
        space::horizontal(),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let search = match app.page {
        Page::Library => {
            Some(text_input("Search the library", &app.search).on_input(Message::Search))
        }
        Page::Workshop => Some(
            text_input("Search the Workshop", &app.browser.query.search)
                .on_input(|text| Message::Shop(Shop::Search(text)))
                .on_submit(Message::Shop(Shop::Find)),
        ),
        Page::Settings => None,
    };
    if let Some(search) = search {
        bar = bar.push(
            search
                .padding([8, 12])
                .size(14)
                .width(260)
                .style(style::field),
        );
    }
    bar.into()
}

fn library(app: &App) -> Element<'_, Message> {
    let items = app.visible();
    let filters = row![
        segmented(
            [
                (Filter::All, "All"),
                (Filter::Pictures, "Pictures"),
                (Filter::Videos, "Videos"),
                (Filter::Shaders, "Shaders"),
                (Filter::Engine, "Wallpaper Engine"),
            ],
            app.filter,
            Message::Filter,
        ),
        space::horizontal(),
        text(format!("{} wallpapers", items.len()))
            .size(13)
            .color(style::FAINT),
        button(text("Rescan").size(13))
            .padding([6, 12])
            .style(style::secondary)
            .on_press(Message::Rescan),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let grid: Element<'_, Message> = if items.is_empty() {
        empty(app)
    } else {
        responsive(move |size| {
            let columns = (((size.width + GAP) / (TILE + GAP)).floor() as usize).max(1);
            let mut rows = Column::new().spacing(GAP);
            for chunk in items.chunks(columns) {
                let mut line = Row::new().spacing(GAP);
                for item in chunk {
                    line = line.push(tile(app, item));
                }
                for _ in chunk.len()..columns {
                    line = line.push(space().width(Fill));
                }
                rows = rows.push(line);
            }
            scrollable(container(rows).padding(iced::padding::right(10)))
                .direction(style::thin())
                .style(style::scroller)
                .height(Fill)
                .into()
        })
        .into()
    };
    row![
        column![filters, grid].spacing(14).width(Fill),
        inspector(app)
    ]
    .spacing(18)
    .height(Fill)
    .into()
}

fn empty(app: &App) -> Element<'_, Message> {
    let folders = app
        .folders()
        .into_iter()
        .take(3)
        .map(|folder| config::tilde(&folder))
        .collect::<Vec<_>>()
        .join(", ");
    container(
        column![
            text("Nothing here yet").size(18).font(style::medium()),
            text(format!(
                "Put pictures, videos or .frag shaders in {folders}, or add a folder in Settings."
            ))
            .size(14)
            .color(style::FAINT),
        ]
        .spacing(8)
        .align_x(Alignment::Center),
    )
    .center(Fill)
    .style(style::sunken)
    .into()
}

fn tile<'a>(app: &'a App, item: &'a Item) -> Element<'a, Message> {
    let picked = app.selected.as_ref() == Some(&item.path);
    let mut layers = stack![preview(app, item, PREVIEW)];
    let shown = app.shown_on(&item.path);
    if !shown.is_empty() {
        layers = layers.push(
            container(
                container(text(shown.join(" · ")).size(11).font(style::medium()))
                    .padding([3, 8])
                    .style(style::pill(style::ACCENT)),
            )
            .padding(8)
            .align_bottom(Fill),
        );
    }
    let caption = row![
        container(
            text(item.name.as_str())
                .size(13)
                .font(style::medium())
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Fill)
        .clip(true),
        text(item.what.label()).size(11).color(style::FAINT),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    button(column![layers, caption].spacing(9))
        .padding(6)
        .width(Fill)
        .style(style::tile(picked))
        .on_press(Message::Select(item.path.clone()))
        .into()
}

fn preview<'a>(app: &'a App, item: &'a Item, height: f32) -> Element<'a, Message> {
    match app.thumbs.get(&item.path) {
        Some(handle) => image(handle.clone())
            .width(Fill)
            .height(height)
            .content_fit(ContentFit::Cover)
            .border_radius(10.0)
            .into(),
        None => container(
            column![
                text(item.what.label()).size(12).color(style::FAINT),
                text(match item.what {
                    What::Shader => "a still appears once it has been shown",
                    _ => "",
                })
                .size(11)
                .color(style::FAINT),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .center_x(Fill)
        .center_y(height)
        .style(style::sunken)
        .into(),
    }
}

fn inspector(app: &App) -> Element<'_, Message> {
    let Some(item) = app.picked() else {
        return container(text("Pick a wallpaper").color(style::FAINT))
            .center_x(INSPECTOR)
            .center_y(Fill)
            .style(style::panel)
            .into();
    };
    let shown = app.shown_on(&item.path);
    let mut details = column![
        preview(app, item, 186.0),
        column![
            text(item.name.as_str()).size(20).font(style::semibold()),
            text(match &item.workshop {
                Some(id) => format!(
                    "{} from Wallpaper Engine · Workshop {id}",
                    item.what.label()
                ),
                None if item.engine => format!("{} from Wallpaper Engine", item.what.label()),
                None => format!(
                    "{} · {}",
                    item.what.label(),
                    config::tilde(item.path.parent().unwrap_or(&item.path))
                ),
            })
            .size(12)
            .color(style::FAINT),
        ]
        .spacing(4),
    ]
    .spacing(16);
    if let Some(id) = &item.workshop {
        details = details.push(
            button(text("Open in the Workshop").size(12))
                .padding([5, 10])
                .style(style::secondary)
                .on_press(Message::Open(workshop::web_page(id))),
        );
    }
    if !shown.is_empty() {
        details = details.push(
            text(format!("On {} now", shown.join(", ")))
                .size(13)
                .color(style::ACCENT),
        );
    }
    details = details.push(section("Show on", monitors(app)));
    if matches!(item.what, What::Picture | What::Video) {
        details = details.push(section(
            "Fit",
            segmented(
                [
                    (Fit::Cover, "Fill the screen"),
                    (Fit::Contain, "Show it whole"),
                ],
                app.fit,
                Message::Fit,
            ),
        ));
    }
    if item.what.missing().is_none() {
        details = details.push(section("Look", look::editor(app, item)));
    }
    let mut apply =
        button(container(text("Set as wallpaper").size(14).font(style::medium())).center_x(Fill))
            .padding([11, 16])
            .width(Fill)
            .style(style::primary);
    if app.daemon.is_some() && item.what.missing().is_none() {
        apply = apply.on_press(Message::Apply);
    }
    let mut bottom = column![].spacing(10);
    if let Some(missing) = item.what.missing() {
        bottom = bottom.push(text(missing).size(12).color(style::SOFT));
    }
    bottom = bottom.push(apply);
    if let Some(notice) = &app.notice {
        bottom = bottom.push(notice_line(notice));
    }
    let details = scrollable(container(details).padding(iced::padding::right(8)))
        .direction(style::thin())
        .style(style::scroller)
        .height(Fill);
    container(column![details, bottom].spacing(14))
        .padding(16)
        .width(INSPECTOR)
        .height(Fill)
        .style(style::panel)
        .into()
}

fn playing(app: &App) -> Element<'_, Message> {
    let Some(daemon) = &app.daemon else {
        return container(
            row![
                container(space())
                    .width(8)
                    .height(8)
                    .style(style::pill(style::DANGER)),
                text("apolline-wall isn't running, so nothing can be set")
                    .size(13)
                    .color(style::SOFT),
                space::horizontal(),
                button(text("Start it").size(13))
                    .padding([6, 14])
                    .style(style::primary)
                    .on_press(Message::Start),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .style(style::panel)
        .into();
    };
    let everything_paused = !daemon.outputs.is_empty() && daemon.outputs.iter().all(|m| m.paused);
    let mut bar = Row::new().spacing(10).align_y(Alignment::Center);
    for monitor in &daemon.outputs {
        let showing = monitor
            .source
            .as_deref()
            .map_or("nothing".to_owned(), |path| app.title(path));
        bar = bar.push(
            container(
                row![
                    text(monitor.name.as_str()).size(12).font(style::semibold()),
                    text(showing).size(12).color(style::SOFT),
                    button(text(if monitor.paused { "Resume" } else { "Pause" }).size(11))
                        .padding([3, 9])
                        .style(style::secondary)
                        .on_press(Message::Pause(Some(monitor.name.clone()), !monitor.paused)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([5, 6])
            .style(style::pill(style::FILL)),
        );
    }
    bar = bar.push(space::horizontal()).push(
        button(
            text(if everything_paused {
                "Resume all"
            } else {
                "Pause all"
            })
            .size(13),
        )
        .padding([6, 14])
        .style(style::secondary)
        .on_press(Message::Pause(None, !everything_paused)),
    );
    container(bar).padding([8, 10]).style(style::panel).into()
}

fn settings(app: &App) -> Element<'_, Message> {
    let defaults = crate::config::defaults();
    let mut folders = Column::new().spacing(6);
    for folder in app.folders() {
        let mut line = row![
            text(config::tilde(&folder)).size(14).width(Fill),
            text(if folder.is_dir() {
                ""
            } else {
                "not created yet"
            })
            .size(12)
            .color(style::FAINT),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        line = if defaults.contains(&folder) {
            line.push(text("always looked in").size(12).color(style::FAINT))
        } else {
            line.push(
                button(text("Remove").size(12))
                    .padding([4, 10])
                    .style(style::secondary)
                    .on_press(Message::RemoveFolder(folder.clone())),
            )
        };
        folders = folders.push(container(line).padding([8, 12]).style(style::sunken));
    }
    let add = row![
        text_input("~/Pictures/More wallpapers", &app.folder)
            .on_input(Message::Folder)
            .on_submit(Message::AddFolder)
            .padding([8, 12])
            .size(14)
            .style(style::field),
        button(text("Add folder").size(13))
            .padding([8, 14])
            .style(style::secondary)
            .on_press(Message::AddFolder),
    ]
    .spacing(8);
    let shaders = column![
        labelled(
            "Frames per second",
            format!("{}", app.fps),
            slider(5..=60, app.fps, Message::Fps)
                .on_release(Message::Limit)
                .style(style::rail)
                .into(),
        ),
        labelled(
            "Resolution",
            format!("{}% of the monitor", (app.scale * 100.0).round()),
            slider(0.25..=1.0, app.scale, Message::Scale)
                .step(0.05_f32)
                .on_release(Message::Limit)
                .style(style::rail)
                .into(),
        ),
        text("Shaders draw every pixel every frame; lower both on a laptop. Pictures and videos ignore these.")
            .size(12)
            .color(style::FAINT),
    ]
    .spacing(14);
    let daemon = match &app.daemon {
        Some(daemon) => text(format!(
            "Running, on {} monitor{}.",
            daemon.outputs.len(),
            if daemon.outputs.len() == 1 { "" } else { "s" }
        )),
        None => {
            text("Not running. Start it from the bar below, or with your session: see the README.")
        }
    };
    let page = column![
        section("Library folders", column![folders, add].spacing(10).into()),
        section("Shaders", shaders.into()),
        section(
            "Steam Workshop",
            row![
                column![
                    text("Show items rated Questionable").size(14),
                    text("Mature items are never shown.")
                        .size(12)
                        .color(style::FAINT),
                ]
                .spacing(3)
                .width(Fill),
                toggler(app.config.questionable)
                    .on_toggle(|on| Message::Shop(Shop::Questionable(on)))
                    .size(22)
                    .style(style::switch),
            ]
            .align_y(Alignment::Center)
            .into()
        ),
        section(
            "Tray",
            row![
                column![
                    text("Show Wallpapers in the tray at login").size(14),
                    text("Pause, resume or pick a random wallpaper for each monitor without the window. Through XDG autostart; on niri or Hyprland add `apolline-wallpapers --tray` to your startup instead.")
                        .size(12)
                        .color(style::FAINT),
                ]
                .spacing(3)
                .width(Fill),
                toggler(app.tray_at_login)
                    .on_toggle(Message::TrayAtLogin)
                    .size(22)
                    .style(style::switch),
            ]
            .spacing(16)
            .align_y(Alignment::Center)
            .into()
        ),
        section("apolline-wall", daemon.size(14).color(style::SOFT).into()),
    ]
    .spacing(26)
    .max_width(720);
    scrollable(container(page).padding([4, 4]))
        .direction(style::thin())
        .style(style::scroller)
        .height(Fill)
        .into()
}

/// Every monitor, or one of them, as a row of choices that scrolls when there are many.
fn monitors(app: &App) -> Element<'_, Message> {
    let mut row = Row::new().spacing(2).push(choice(
        "Every monitor",
        app.target.is_none(),
        Message::Target(None),
    ));
    for monitor in app.daemon.iter().flat_map(|daemon| &daemon.outputs) {
        let on = app.target.as_deref() == Some(monitor.name.as_str());
        row = row.push(choice(
            &monitor.name,
            on,
            Message::Target(Some(monitor.name.clone())),
        ));
    }
    container(scrollable(row).direction(scrollable::Direction::Horizontal(
        scrollable::Scrollbar::new().width(0).scroller_width(0),
    )))
    .padding(3)
    .style(style::segments)
    .into()
}

/// The last problem, dismissed with a click.
fn notice_line(notice: &str) -> Element<'_, Message> {
    button(text(notice).size(12))
        .padding([8, 10])
        .width(Fill)
        .style(|theme, _| iced::widget::button::Style {
            background: style::notice(theme).background,
            text_color: style::TEXT,
            border: style::notice(theme).border,
            ..Default::default()
        })
        .on_press(Message::Dismiss)
        .into()
}

fn section<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    column![
        text(title)
            .size(13)
            .font(style::semibold())
            .color(style::SOFT),
        body
    ]
    .spacing(10)
    .into()
}

fn labelled<'a>(
    label: &'a str,
    value: String,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        row![
            text(label).size(14),
            space::horizontal(),
            text(value).size(13).color(style::SOFT)
        ],
        control
    ]
    .spacing(8)
    .into()
}

fn choice<'a>(label: &str, on: bool, message: Message) -> Element<'a, Message> {
    button(text(label.to_owned()).size(13))
        .padding([6, 12])
        .style(style::choice(on))
        .on_press(message)
        .into()
}

fn segmented<'a, T: Copy + PartialEq + 'a, const N: usize>(
    options: [(T, &'a str); N],
    current: T,
    on: impl Fn(T) -> Message,
) -> Element<'a, Message> {
    let buttons = Row::with_children(options.map(|(value, label)| {
        button(text(label).size(13))
            .padding([6, 14])
            .style(style::choice(value == current))
            .on_press(on(value))
            .into()
    }))
    .spacing(2);
    container(buttons)
        .padding(3)
        .style(style::segments)
        .width(Length::Shrink)
        .into()
}
