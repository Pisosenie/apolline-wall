//! Where a monitor's surface lives. Almost every compositor has wlr-layer-shell's background
//! layer. GNOME has none, so there each monitor gets an ordinary window that apolline-wall's
//! GNOME Shell extension moves behind everything; without the extension it would cover the
//! desktop, so it isn't opened at all.

use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};

use super::App;

/// The extension's id, as GNOME knows it.
pub const GNOME_EXTENSION: &str = "apolline-wall@pisosenie.github.io";
/// What the extension looks for: this app id, and the monitor's connector after a colon in
/// the title.
const APP_ID: &str = "apolline-wall";

pub(super) enum Shells {
    Layers(LayerShell),
    Windows(XdgShell),
}

pub(super) enum Shell {
    Layer(LayerSurface),
    Window(Window),
}

impl Shell {
    pub fn wl_surface(&self) -> &WlSurface {
        match self {
            Shell::Layer(layer) => layer.wl_surface(),
            Shell::Window(window) => window.wl_surface(),
        }
    }
}

pub(super) fn bind(globals: &GlobalList, qh: &QueueHandle<App>) -> Result<Shells, String> {
    if let Ok(layers) = LayerShell::bind(globals, qh) {
        return Ok(Shells::Layers(layers));
    }
    if !on_gnome() {
        return Err(
            "this compositor has no wlr-layer-shell, so there is no background layer to draw on"
                .into(),
        );
    }
    if !extension_enabled() {
        return Err(format!(
            "on GNOME it draws through its Shell extension, which isn't enabled: run `gnome-extensions enable {GNOME_EXTENSION}`, then log out and back in"
        ));
    }
    XdgShell::bind(globals, qh)
        .map(Shells::Windows)
        .map_err(|err| format!("xdg-shell: {err}"))
}

fn on_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktops| desktops.split(':').any(|desktop| desktop == "GNOME"))
}

fn extension_enabled() -> bool {
    let read = |key: &str| {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.shell", key])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let listed = read("enabled-extensions").is_some_and(|list| list.contains(GNOME_EXTENSION));
    let all_off = read("disable-user-extensions").is_some_and(|off| off.trim() == "true");
    listed && !all_off
}

impl App {
    /// A background surface for one monitor. The compositor answers its first commit with a
    /// configure, which is where drawing starts.
    pub(super) fn open_surface(
        &self,
        qh: &QueueHandle<Self>,
        output: &WlOutput,
        name: &str,
    ) -> (Shell, Option<WpViewport>) {
        let wayland = self.wayland();
        let surface = wayland.compositor.create_surface(qh);
        let viewport = wayland
            .viewporter
            .as_ref()
            .and_then(|viewporter| viewporter.get().ok())
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        let shell = match &wayland.shells {
            Shells::Layers(layers) => {
                let layer = layers.create_layer_surface(
                    qh,
                    surface,
                    Layer::Background,
                    Some("apolline-wallpaper"),
                    Some(output),
                );
                layer.set_anchor(Anchor::all());
                layer.set_exclusive_zone(-1);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.commit();
                Shell::Layer(layer)
            }
            Shells::Windows(xdg) => {
                // Clicks go through to whatever is behind, as they would on a desktop.
                if let Ok(region) = Region::new(&wayland.compositor) {
                    surface.set_input_region(Some(region.wl_region()));
                }
                let window = xdg.create_window(surface, WindowDecorations::None, qh);
                window.set_app_id(APP_ID);
                window.set_title(format!("{APP_ID}:{name}"));
                window.commit();
                Shell::Window(window)
            }
        };
        (shell, viewport)
    }

    /// The monitor's size in logical pixels, for a window the compositor left to choose.
    fn output_size(&self, index: usize) -> Option<(u32, u32)> {
        let output = self.output_at(index)?;
        let info = self.wayland().outputs.info(&output.output)?;
        let (w, h) = info.logical_size?;
        (w > 0 && h > 0).then_some((w as u32, h as u32))
    }
}

impl WindowHandler for App {
    /// Only the extension or the user's own tools would close it; it comes back as it was.
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let Some(index) = self.screen_of(window.wl_surface()) else {
            return;
        };
        let size = match configure.new_size {
            (Some(w), Some(h)) => Some((w.get(), h.get())),
            _ => self.output_size(index),
        };
        if let Some(size) = size {
            self.configured(index, size);
        }
    }
}
