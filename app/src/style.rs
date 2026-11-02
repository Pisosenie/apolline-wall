use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, container, scrollable, slider, text_input, toggler};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, font};

/// Apolline's look, so the app sits next to its shell's own windows.
const fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn white(alpha: f32) -> Color {
    Color::from_rgba(1.0, 1.0, 1.0, alpha)
}

pub const INK: Color = rgb(0x0c0d10);
pub const SURFACE: Color = rgb(0x15171b);
pub const SUNKEN: Color = white(0.04);
pub const FILL: Color = white(0.07);
pub const HOVER: Color = white(0.11);
pub const HAIRLINE: Color = white(0.08);
pub const TEXT: Color = rgb(0xf4f3f0);
pub const SOFT: Color = Color::from_rgba(0.957, 0.953, 0.941, 0.86);
pub const FAINT: Color = Color::from_rgba(0.957, 0.953, 0.941, 0.5);
pub const ACCENT: Color = rgb(0x86bdf0);
pub const ON_ACCENT: Color = rgb(0x0b1422);
pub const DANGER: Color = rgb(0xff7a6b);
pub const RADIUS: f32 = 14.0;
pub const CONTROL: f32 = 10.0;
pub const PILL: f32 = 999.0;

pub const UI: &str = "Geist";

pub fn medium() -> Font {
    Font {
        weight: font::Weight::Medium,
        ..Font::with_name(UI)
    }
}

pub fn semibold() -> Font {
    Font {
        weight: font::Weight::Semibold,
        ..Font::with_name(UI)
    }
}

fn rounded(radius: f32) -> Border {
    Border {
        radius: radius.into(),
        ..Border::default()
    }
}

fn outlined(color: Color, width: f32, radius: f32) -> Border {
    Border {
        color,
        width,
        radius: radius.into(),
    }
}

pub fn window(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(INK)),
        ..container::Style::default()
    }
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: outlined(HAIRLINE, 1.0, RADIUS + 4.0),
        ..container::Style::default()
    }
}

pub fn sunken(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SUNKEN)),
        border: outlined(HAIRLINE, 1.0, RADIUS),
        ..container::Style::default()
    }
}

pub fn pill(background: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(background)),
        border: rounded(PILL),
        ..container::Style::default()
    }
}

pub fn notice(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(Color { a: 0.16, ..DANGER })),
        border: outlined(Color { a: 0.4, ..DANGER }, 1.0, CONTROL),
        ..container::Style::default()
    }
}

/// A wallpaper in the grid: an accent ring when picked, a soft one when hovered.
pub fn tile(picked: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (edge, width) = match (picked, hovered) {
            (true, _) => (ACCENT, 2.0),
            (false, true) => (white(0.22), 1.0),
            (false, false) => (HAIRLINE, 1.0),
        };
        button::Style {
            background: Some(Background::Color(if hovered { FILL } else { SUNKEN })),
            text_color: TEXT,
            border: outlined(edge, width, RADIUS),
            shadow: if picked {
                Shadow {
                    color: Color { a: 0.28, ..ACCENT },
                    offset: Vector::ZERO,
                    blur_radius: 18.0,
                }
            } else {
                Shadow::default()
            },
            ..button::Style::default()
        }
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Color {
            r: ACCENT.r + (1.0 - ACCENT.r) * 0.14,
            g: ACCENT.g + (1.0 - ACCENT.g) * 0.14,
            b: ACCENT.b + (1.0 - ACCENT.b) * 0.14,
            a: 1.0,
        },
        button::Status::Disabled => FILL,
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if status == button::Status::Disabled {
            FAINT
        } else {
            ON_ACCENT
        },
        border: rounded(CONTROL + 2.0),
        ..button::Style::default()
    }
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => HOVER,
            _ => FILL,
        })),
        text_color: if status == button::Status::Disabled {
            FAINT
        } else {
            TEXT
        },
        border: rounded(CONTROL + 2.0),
        ..button::Style::default()
    }
}

/// One option of a segmented choice.
pub fn choice(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: match (on, hovered) {
                (true, _) => Some(Background::Color(white(0.14))),
                (false, true) => Some(Background::Color(FILL)),
                (false, false) => None,
            },
            text_color: if on { TEXT } else { SOFT },
            border: rounded(CONTROL),
            ..button::Style::default()
        }
    }
}

pub fn segments(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SUNKEN)),
        border: outlined(HAIRLINE, 1.0, CONTROL + 3.0),
        ..container::Style::default()
    }
}

pub fn field(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => Color { a: 0.7, ..ACCENT },
        text_input::Status::Hovered => white(0.16),
        _ => HAIRLINE,
    };
    text_input::Style {
        background: Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.28)),
        border: outlined(edge, 1.0, CONTROL + 2.0),
        icon: FAINT,
        placeholder: FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

pub fn rail(theme: &Theme, status: slider::Status) -> slider::Style {
    let mut style = slider::default(theme, status);
    style.rail.backgrounds = (Background::Color(ACCENT), Background::Color(FILL));
    style.rail.width = 4.0;
    style.handle.background = Background::Color(TEXT);
    style
}

pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, hovered) = match status {
        toggler::Status::Active { is_toggled } | toggler::Status::Disabled { is_toggled } => {
            (is_toggled, false)
        }
        toggler::Status::Hovered { is_toggled } => (is_toggled, true),
    };
    let track = match (on, hovered) {
        (true, _) => ACCENT,
        (false, false) => white(0.16),
        (false, true) => white(0.22),
    };
    toggler::Style {
        background: Background::Color(track),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: Background::Color(TEXT),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.12,
    }
}

pub fn thin() -> Direction {
    Direction::Vertical(Scrollbar::new().width(4).scroller_width(4).spacing(8))
}

pub fn scroller(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    let grip = match status {
        scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. } => 0.24,
        _ => 0.12,
    };
    for rail in [&mut style.vertical_rail, &mut style.horizontal_rail] {
        rail.background = None;
        rail.border = Border::default();
        rail.scroller.background = Background::Color(white(grip));
        rail.scroller.border = rounded(PILL);
    }
    style
}
