//! The Wayland side: a surface per output on wlr-layer-shell's background layer, or on GNOME
//! a window its Shell extension puts behind everything, with frame callbacks that pace
//! drawing, and where the pointer is over them for scenes that follow it.

mod shell;

use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::calloop::LoopHandle;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{Transform, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::{
    self, WpViewport,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState, SimpleGlobal};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_simple, delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};

use super::{App, Desktop, Surface};
use crate::video::Platform;

pub(super) struct Wayland {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    shells: shell::Shells,
    viewporter: Option<SimpleGlobal<WpViewporter, 1>>,
    seats: SeatState,
    /// Each seat's pointer, by the seat it's on.
    pointers: Vec<(WlSeat, WlPointer)>,
    qh: QueueHandle<App>,
}

/// One output's surface.
pub(super) struct Output {
    output: WlOutput,
    shell: shell::Shell,
    viewport: Option<WpViewport>,
    /// Its size in logical pixels, once the compositor has said.
    logical: Option<(u32, u32)>,
}

pub(super) fn connect(
    handle: &LoopHandle<'static, App>,
) -> Result<(Wayland, RawDisplayHandle, Platform), String> {
    let conn = Connection::connect_to_env().map_err(|err| format!("Wayland: {err}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|err| err.to_string())?;
    let qh = queue.handle();
    let shells = shell::bind(&globals, &qh)?;
    let wayland = Wayland {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)
            .map_err(|err| format!("wl_compositor: {err}"))?,
        shells,
        viewporter: SimpleGlobal::bind(&globals, &qh).ok(),
        seats: SeatState::new(&globals, &qh),
        pointers: Vec::new(),
        qh,
    };
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|err| err.to_string())?;
    let display: *mut c_void = conn.backend().display_ptr().cast();
    let raw = WaylandDisplayHandle::new(NonNull::new(display).ok_or("no wl_display")?);
    Ok((
        wayland,
        RawDisplayHandle::Wayland(raw),
        Platform::Wayland(display),
    ))
}

impl App {
    /// Wayland's state; Wayland events only ever come with a Wayland connection.
    fn wayland(&self) -> &Wayland {
        match &self.desktop {
            Desktop::Wayland(wayland) => wayland,
            Desktop::X11(_) => unreachable!("a Wayland event without a Wayland connection"),
        }
    }

    fn wayland_mut(&mut self) -> &mut Wayland {
        match &mut self.desktop {
            Desktop::Wayland(wayland) => wayland,
            Desktop::X11(_) => unreachable!("a Wayland event without a Wayland connection"),
        }
    }

    fn output_at(&self, index: usize) -> Option<&Output> {
        match &self.screens[index].surface {
            Surface::Wayland(output) => Some(output),
            Surface::X11(_) => None,
        }
    }

    fn output_at_mut(&mut self, index: usize) -> Option<&mut Output> {
        match &mut self.screens[index].surface {
            Surface::Wayland(output) => Some(output),
            Surface::X11(_) => None,
        }
    }

    /// Asks for a callback when the compositor wants the next frame; it has to go out before
    /// the commit it belongs to.
    pub(super) fn wayland_frame(&self, index: usize) {
        if let Some(output) = self.output_at(index) {
            let surface = output.shell.wl_surface();
            surface.frame(&self.wayland().qh, surface.clone());
        }
    }

    /// The output's physical mode when the viewporter can scale to it, otherwise its logical
    /// size times its integer scale.
    pub(super) fn wayland_buffer_size(&self, index: usize) -> Option<(u32, u32)> {
        let output = self.output_at(index)?;
        let (w, h) = output.logical?;
        let wayland = self.wayland();
        let info = wayland.outputs.info(&output.output)?;
        let scale = info.scale_factor.max(1) as u32;
        let physical = wayland
            .viewporter
            .as_ref()
            .and_then(|_| current_mode(&info));
        Some(physical.unwrap_or((w * scale, h * scale)))
    }

    /// Sizes the surface for its buffer and says what EGL should draw into.
    pub(super) fn wayland_prepare(&self, index: usize) -> Option<RawWindowHandle> {
        let output = self.output_at(index)?;
        let (w, h) = output.logical?;
        let wayland = self.wayland();
        let scale = wayland
            .outputs
            .info(&output.output)
            .map_or(1, |info| info.scale_factor.max(1));
        let surface = output.shell.wl_surface();
        match &output.viewport {
            Some(viewport) => viewport.set_destination(w as i32, h as i32),
            None => surface.set_buffer_scale(scale),
        }
        if let Ok(region) = Region::new(&wayland.compositor) {
            region.add(0, 0, w as i32, h as i32);
            surface.set_opaque_region(Some(region.wl_region()));
        }
        let handle = WaylandWindowHandle::new(NonNull::new(surface.id().as_ptr().cast())?);
        Some(RawWindowHandle::Wayland(handle))
    }

    pub(super) fn wayland_close(&self, output: Output) {
        if let Some(viewport) = output.viewport {
            viewport.destroy();
        }
    }

    /// Unmaps the surface so another program's, on the same layer, is what shows.
    pub(super) fn wayland_hide(&self, index: usize) {
        if let Some(output) = self.output_at(index) {
            let surface = output.shell.wl_surface();
            surface.attach(None, 0, 0);
            surface.commit();
        }
    }

    /// Comes back on a fresh surface: compositors differ on mapping one that was unmapped,
    /// but every one configures a new surface, and drawing resumes from there.
    pub(super) fn wayland_reopen(&mut self, index: usize) {
        let Some(output) = self.output_at(index).map(|output| output.output.clone()) else {
            return;
        };
        let name = self.screens[index].name.clone();
        let (shell, viewport) = self.open_surface(&self.wayland().qh, &output, &name);
        if let Some(surface) = self.output_at_mut(index) {
            if let Some(old) = std::mem::replace(&mut surface.viewport, viewport) {
                old.destroy();
            }
            surface.shell = shell;
            surface.logical = None;
        }
    }

    fn screen_of(&self, surface: &WlSurface) -> Option<usize> {
        (0..self.screens.len()).find(|&index| {
            self.output_at(index)
                .is_some_and(|output| output.shell.wl_surface() == surface)
        })
    }

    fn screen_on(&self, wanted: &WlOutput) -> Option<usize> {
        (0..self.screens.len()).find(|&index| {
            self.output_at(index)
                .is_some_and(|output| output.output == *wanted)
        })
    }

    fn configured(&mut self, index: usize, size: (u32, u32)) {
        if let Some(output) = self.output_at_mut(index) {
            output.logical = Some(size);
        }
        self.place(index);
    }
}

/// What the compositor calls the monitor itself: its description without the connector, which
/// usually holds a serial, or else its make and model.
fn identity(info: &OutputInfo) -> Option<String> {
    let described = info.description.as_deref().map(|description| {
        info.name
            .as_deref()
            .and_then(|name| description.strip_suffix(&format!(" ({name})")))
            .unwrap_or(description)
            .trim()
            .to_owned()
    });
    let made = format!("{} {}", info.make, info.model).trim().to_owned();
    described
        .filter(|described| !described.is_empty())
        .or_else(|| (!made.is_empty() && info.make != "Unknown").then_some(made))
}

fn current_mode(info: &OutputInfo) -> Option<(u32, u32)> {
    let mode = info.modes.iter().find(|mode| mode.current)?;
    let (w, h) = (
        mode.dimensions.0.max(1) as u32,
        mode.dimensions.1.max(1) as u32,
    );
    Some(match info.transform {
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270 => (h, w),
        _ => (w, h),
    })
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &WlSurface,
        _: i32,
    ) {
        if let Some(index) = self.screen_of(surface) {
            self.place(index);
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        if let Some(index) = self.screen_of(surface) {
            self.frame_done(index);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.wayland_mut().outputs
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {
        let info = self.wayland().outputs.info(&output);
        let identity = info.as_ref().and_then(identity);
        let name = info
            .and_then(|info| info.name)
            .unwrap_or_else(|| format!("output-{}", output.id().protocol_id()));
        let (shell, viewport) = self.open_surface(qh, &output, &name);
        self.add_screen(
            name,
            Surface::Wayland(Output {
                output,
                shell,
                viewport,
                logical: None,
            }),
            identity,
        );
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        if let Some(index) = self.screen_on(&output) {
            self.place(index);
        }
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        if let Some(index) = self.screen_on(&output) {
            self.forget(index);
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(index) = self.screen_of(layer.wl_surface()) {
            self.forget(index);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(index) = self.screen_of(layer.wl_surface()) else {
            return;
        };
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            self.configured(index, (w, h));
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.wayland_mut().seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        let wayland = self.wayland_mut();
        if capability == Capability::Pointer
            && !wayland.pointers.iter().any(|(on, _)| *on == seat)
            && let Ok(pointer) = wayland.seats.get_pointer(qh, &seat)
        {
            wayland.pointers.push((seat, pointer));
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.wayland_mut().forget_pointer(&seat);
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.wayland_mut().forget_pointer(&seat);
    }
}

impl Wayland {
    fn forget_pointer(&mut self, seat: &WlSeat) {
        self.pointers.retain(|(on, pointer)| {
            if on == seat && pointer.version() >= 3 {
                pointer.release();
            }
            on != seat
        });
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if !matches!(
                event.kind,
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. }
            ) {
                continue;
            }
            let Some(index) = self.screen_of(&event.surface) else {
                continue;
            };
            if let Some(size) = self.output_at(index).and_then(|output| output.logical) {
                self.pointed(index, event.position, size);
            }
        }
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.wayland_mut().registry
    }

    registry_handlers![OutputState, SeatState];
}

impl Dispatch<WpViewport, ()> for App {
    fn event(
        _: &mut Self,
        _: &WpViewport,
        _: wp_viewport::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);
delegate_registry!(App);
delegate_seat!(App);
delegate_pointer!(App);
delegate_simple!(App, WpViewporter, 1);
