//! The X11 side, for X11 sessions and for Wayland sessions with no background to offer: a
//! desktop-type window per RandR monitor, which window managers keep under everything, drawn
//! with EGL through Xlib, and a timer where Wayland would send frame callbacks.

use std::ffi::c_void;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::ptr::NonNull;
use std::time::Duration;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle};
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{
    Interest, LoopHandle, Mode as Trigger, PostAction,
};
use x11_dl::xlib::Xlib;
use x11_dl::xlib_xcb::{XEventQueueOwner, Xlib_xcb};
use x11rb::connection::Connection;
use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    StackMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

use super::{App, Desktop, Surface};
use crate::protocol::{self, Event};
use crate::video::Platform;

/// How long a frame waits where Wayland would say when to draw the next.
const FRAME: Duration = Duration::from_millis(16);
const CLASS: &[u8] = b"apolline-wall\0apolline-wall\0";

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DESKTOP,
        _NET_WM_STATE,
        _NET_WM_STATE_BELOW,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_DESKTOP,
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

pub(super) struct X11 {
    conn: XCBConnection,
    root: u32,
    root_size: (u16, u16),
    atoms: Atoms,
    // The libraries the connection came from stay loaded as long as it lives.
    _xlib: Xlib,
    _xlib_xcb: Xlib_xcb,
}

pub(super) struct Window {
    id: u32,
    position: (i16, i16),
    size: (u32, u32),
}

impl Window {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn handle(&self) -> RawWindowHandle {
        RawWindowHandle::Xlib(XlibWindowHandle::new(self.id.into()))
    }
}

struct Monitor {
    name: String,
    position: (i16, i16),
    size: (u16, u16),
}

/// Opens the display through Xlib, which EGL needs, and talks over its XCB connection.
pub(super) fn connect(
    handle: &LoopHandle<'static, App>,
) -> Result<(X11, RawDisplayHandle, Platform), String> {
    let xlib = Xlib::open().map_err(|err| format!("Xlib: {err}"))?;
    let xlib_xcb = Xlib_xcb::open().map_err(|err| format!("Xlib-xcb: {err}"))?;
    let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
    let Some(display) = NonNull::new(display) else {
        return Err("X11: the display doesn't open".into());
    };
    let screen = unsafe { (xlib.XDefaultScreen)(display.as_ptr()) };
    let conn = unsafe {
        (xlib_xcb.XSetEventQueueOwner)(display.as_ptr(), XEventQueueOwner::XCBOwnsEventQueue);
        let xcb = (xlib_xcb.XGetXCBConnection)(display.as_ptr());
        XCBConnection::from_raw_xcb_connection(xcb.cast(), false)
    }
    .map_err(|err| format!("X11: {err}"))?;
    let x_screen = conn
        .setup()
        .roots
        .get(screen as usize)
        .ok_or("X11: no such screen")?;
    let (root, root_size) = (
        x_screen.root,
        (x_screen.width_in_pixels, x_screen.height_in_pixels),
    );
    let atoms = Atoms::new(&conn)
        .map_err(|err| format!("X11 atoms: {err}"))?
        .reply()
        .map_err(|err| format!("X11 atoms: {err}"))?;
    // Monitors coming and going arrive as RandR events on the root window.
    if conn
        .randr_query_version(1, 5)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some()
    {
        let _ = conn.randr_select_input(root, randr::NotifyMask::SCREEN_CHANGE);
    }
    let readable = unsafe { BorrowedFd::borrow_raw(conn.as_raw_fd()) }
        .try_clone_to_owned()
        .map_err(|err| format!("X11: {err}"))?;
    handle
        .insert_source(
            Generic::new(readable, Interest::READ, Trigger::Level),
            |_, _, app: &mut App| {
                app.x11_events();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| err.to_string())?;
    let raw = XlibDisplayHandle::new(Some(display.cast::<c_void>()), screen);
    let x11 = X11 {
        conn,
        root,
        root_size,
        atoms,
        _xlib: xlib,
        _xlib_xcb: xlib_xcb,
    };
    Ok((
        x11,
        RawDisplayHandle::Xlib(raw),
        Platform::X11(display.as_ptr().cast()),
    ))
}

impl X11 {
    pub fn flush(&self) {
        let _ = self.conn.flush();
    }

    /// RandR's monitors, or the whole screen as one when RandR can't say.
    fn monitors(&self) -> Vec<Monitor> {
        let listed = self
            .conn
            .randr_get_monitors(self.root, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.monitors)
            .unwrap_or_default();
        let monitors: Vec<Monitor> = listed
            .iter()
            .map(|monitor| Monitor {
                name: self
                    .atom_name(monitor.name)
                    .unwrap_or_else(|| format!("monitor-{}", monitor.name)),
                position: (monitor.x, monitor.y),
                size: (monitor.width, monitor.height),
            })
            .collect();
        if monitors.is_empty() {
            return vec![Monitor {
                name: "screen".into(),
                position: (0, 0),
                size: self.root_size,
            }];
        }
        monitors
    }

    fn atom_name(&self, atom: u32) -> Option<String> {
        let reply = self.conn.get_atom_name(atom).ok()?.reply().ok()?;
        Some(String::from_utf8_lossy(&reply.name).into_owned())
    }

    /// A desktop window over one monitor: under every window, on every workspace, and out
    /// of taskbars and pagers.
    fn open(&self, monitor: &Monitor) -> Result<Window, String> {
        let fail = |err: &dyn std::fmt::Display| format!("X11 window on {}: {err}", monitor.name);
        let id = self.conn.generate_id().map_err(|err| fail(&err))?;
        let (x, y) = monitor.position;
        let (width, height) = monitor.size;
        let atoms = &self.atoms;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY);
        self.conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                id,
                self.root,
                x,
                y,
                width,
                height,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux,
            )
            .map_err(|err| fail(&err))?;
        let title = format!("apolline-wall:{}", monitor.name);
        let states = [
            atoms._NET_WM_STATE_BELOW,
            atoms._NET_WM_STATE_STICKY,
            atoms._NET_WM_STATE_SKIP_TASKBAR,
            atoms._NET_WM_STATE_SKIP_PAGER,
        ];
        let properties = [
            self.conn.change_property32(
                PropMode::REPLACE,
                id,
                atoms._NET_WM_WINDOW_TYPE,
                AtomEnum::ATOM,
                &[atoms._NET_WM_WINDOW_TYPE_DESKTOP],
            ),
            self.conn.change_property32(
                PropMode::REPLACE,
                id,
                atoms._NET_WM_STATE,
                AtomEnum::ATOM,
                &states,
            ),
            self.conn.change_property32(
                PropMode::REPLACE,
                id,
                atoms._NET_WM_DESKTOP,
                AtomEnum::CARDINAL,
                &[u32::MAX],
            ),
            self.conn.change_property8(
                PropMode::REPLACE,
                id,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                CLASS,
            ),
            self.conn.change_property8(
                PropMode::REPLACE,
                id,
                atoms._NET_WM_NAME,
                atoms.UTF8_STRING,
                title.as_bytes(),
            ),
        ];
        for property in properties {
            property.map_err(|err| fail(&err))?;
        }
        // Where it goes, stated as the user's wish, which window managers honour.
        let hints = WmSizeHints {
            position: Some((WmSizeHintsSpecification::UserSpecified, x.into(), y.into())),
            size: Some((
                WmSizeHintsSpecification::UserSpecified,
                width.into(),
                height.into(),
            )),
            ..WmSizeHints::default()
        };
        hints
            .set_normal_hints(&self.conn, id)
            .map_err(|err| fail(&err))?;
        self.conn.map_window(id).map_err(|err| fail(&err))?;
        let window = Window {
            id,
            position: monitor.position,
            size: (width.into(), height.into()),
        };
        self.place(&window).map_err(|err| fail(&err))?;
        self.flush();
        Ok(window)
    }

    fn place(&self, window: &Window) -> Result<(), x11rb::errors::ConnectionError> {
        let aux = ConfigureWindowAux::new()
            .x(i32::from(window.position.0))
            .y(i32::from(window.position.1))
            .width(window.size.0)
            .height(window.size.1)
            .stack_mode(StackMode::BELOW);
        self.conn.configure_window(window.id, &aux).map(drop)
    }
}

impl App {
    fn x11(&self) -> Option<&X11> {
        match &self.desktop {
            Desktop::X11(x11) => Some(x11),
            Desktop::Wayland(_) => None,
        }
    }

    fn screen_of_window(&self, id: u32) -> Option<usize> {
        self.screens
            .iter()
            .position(|screen| matches!(&screen.surface, Surface::X11(window) if window.id == id))
    }

    /// Opens a window for each new monitor, moves the ones that moved, and closes the ones
    /// that went.
    pub(super) fn x11_monitors(&mut self) {
        let Some(x11) = self.x11() else {
            return;
        };
        let found = x11.monitors();
        let gone: Vec<u64> = self
            .screens
            .iter()
            .filter(|screen| !found.iter().any(|monitor| monitor.name == screen.name))
            .map(|screen| screen.id)
            .collect();
        for id in gone {
            if let Some(index) = self.index_of(id) {
                self.forget(index);
            }
        }
        for monitor in found {
            match self
                .screens
                .iter()
                .position(|screen| screen.name == monitor.name)
            {
                Some(index) => self.x11_moved(index, &monitor),
                None => {
                    let opened = self.x11().map(|x11| x11.open(&monitor));
                    match opened {
                        Some(Ok(window)) => {
                            let index = self.add_screen(monitor.name, Surface::X11(window));
                            self.place(index);
                        }
                        Some(Err(message)) => protocol::emit(&Event::Error {
                            output: Some(monitor.name),
                            source: None,
                            message,
                        }),
                        None => {}
                    }
                }
            }
        }
    }

    fn x11_moved(&mut self, index: usize, monitor: &Monitor) {
        let size = (u32::from(monitor.size.0), u32::from(monitor.size.1));
        let Surface::X11(window) = &mut self.screens[index].surface else {
            return;
        };
        if window.position == monitor.position && window.size == size {
            return;
        }
        window.position = monitor.position;
        window.size = size;
        if let (Desktop::X11(x11), Surface::X11(window)) =
            (&self.desktop, &self.screens[index].surface)
        {
            let _ = x11.place(window);
        }
        self.place(index);
    }

    pub(super) fn x11_events(&mut self) {
        let Some(x11) = self.x11() else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(Some(event)) = x11.conn.poll_for_event() {
            events.push(event);
        }
        let mut monitors_changed = false;
        for event in events {
            match event {
                XEvent::Expose(exposed) if exposed.count == 0 => {
                    if let Some(index) = self.screen_of_window(exposed.window) {
                        self.screens[index].stale = true;
                        if !self.screens[index].frame_pending {
                            self.draw(index);
                        }
                    }
                }
                XEvent::ConfigureNotify(configured) => {
                    let size = (u32::from(configured.width), u32::from(configured.height));
                    if let Some(index) = self.screen_of_window(configured.window)
                        && let Surface::X11(window) = &mut self.screens[index].surface
                        && window.size != size
                        && size.0 > 0
                        && size.1 > 0
                    {
                        window.size = size;
                        self.place(index);
                    }
                }
                XEvent::RandrScreenChangeNotify(_) => monitors_changed = true,
                _ => {}
            }
        }
        if monitors_changed {
            self.x11_monitors();
        }
    }

    /// A frame after a short wait, since X11 doesn't say when the next one is wanted.
    pub(super) fn x11_frame(&mut self, index: usize) {
        let id = self.screens[index].id;
        let timer = Timer::from_duration(FRAME);
        let _ = self
            .handle
            .insert_source(timer, move |_, _, app: &mut App| {
                if let Some(index) = app.index_of(id) {
                    app.frame_done(index);
                }
                TimeoutAction::Drop
            });
    }

    pub(super) fn x11_hide(&self, id: u32) {
        if let Some(x11) = self.x11() {
            let _ = x11.conn.unmap_window(id);
            x11.flush();
        }
    }

    pub(super) fn x11_show(&self, id: u32) {
        if let Some(x11) = self.x11() {
            let _ = x11.conn.map_window(id);
            x11.flush();
        }
    }

    pub(super) fn x11_close(&self, window: Window) {
        if let Some(x11) = self.x11() {
            let _ = x11.conn.destroy_window(window.id);
            x11.flush();
        }
    }
}
