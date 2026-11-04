// vpointer X Y [click|scroll N]: moves a virtual pointer on a 1440x900 output and acts.
use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1 as Manager;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1 as Pointer;

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<Manager, ()> for State {
    fn event(
        _: &mut Self,
        _: &Manager,
        _: <Manager as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<Pointer, ()> for State {
    fn event(
        _: &mut Self,
        _: &Pointer,
        _: <Pointer as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (x, y): (u32, u32) = (args[0].parse().unwrap(), args[1].parse().unwrap());
    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut queue) = registry_queue_init::<State>(&conn).unwrap();
    let qh = queue.handle();
    let manager: Manager = globals.bind(&qh, 1..=2, ()).unwrap();
    let pointer = manager.create_virtual_pointer(None, &qh, ());
    let pause = |queue: &mut wayland_client::EventQueue<State>, ms| {
        queue.roundtrip(&mut State).unwrap();
        std::thread::sleep(Duration::from_millis(ms));
    };
    pause(&mut queue, 150);
    pointer.motion_absolute(0, x, y, 1440, 900);
    pointer.frame();
    pause(&mut queue, 250);
    match args.get(2).map(String::as_str) {
        Some("click") => {
            pointer.button(1, 0x110, wl_pointer::ButtonState::Pressed);
            pointer.frame();
            pause(&mut queue, 80);
            pointer.button(2, 0x110, wl_pointer::ButtonState::Released);
            pointer.frame();
        }
        Some("scroll") => {
            let steps: f64 = args[3].parse().unwrap();
            pointer.axis(1, wl_pointer::Axis::VerticalScroll, steps * 15.0);
            pointer.frame();
        }
        _ => {}
    }
    pause(&mut queue, 300);
    pointer.destroy();
    queue.roundtrip(&mut State).unwrap();
}
