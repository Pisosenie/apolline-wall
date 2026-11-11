//! Plasma draws its desktop, icons and all, over the layer a wallpaper would use, so there the
//! wallpaper plugin in `plasma/` shows a monitor instead. The monitor's frames are drawn
//! offscreen, read back and sent to a PipeWire stream, which the plugin plays inside
//! plasmashell. The plugin asks for the stream with `apolline-wall feed`, and from then on the
//! monitor belongs to it: under Plasma a surface of our own would only cover the desktop.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use pipewire as pw;
use pw::spa;
use pw::spa::param::ParamType;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::VideoFormat;
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle, SpaTypes};
use pw::stream::{StreamFlags, StreamState};
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{
    Interest, LoopHandle, Mode, PostAction, RegistrationToken,
};

use super::{App, Desktop};
use crate::gl::{Gl, Target};
use crate::protocol::{self, Event, FeedStream};

const FRAME: Duration = Duration::from_millis(16);

pub(super) struct Pipewire {
    core: pw::core::CoreRc,
    _context: pw::context::ContextRc,
    handle: LoopHandle<'static, App>,
    token: RegistrationToken,
}

impl Pipewire {
    fn connect(handle: &LoopHandle<'static, App>) -> Result<Self, String> {
        let failed = |err: pw::Error| format!("PipeWire: {err}");
        pw::init();
        let main = pw::loop_::LoopRc::new(None).map_err(failed)?;
        let context = pw::context::ContextRc::new(&main, None).map_err(failed)?;
        let core = context.connect_rc(None).map_err(failed)?;
        let fd = main
            .fd()
            .try_clone_to_owned()
            .map_err(|err| format!("PipeWire: {err}"))?;
        let token = handle
            .insert_source(
                Generic::new(fd, Interest::READ, Mode::Level),
                move |_, _, _| {
                    main.iterate(pw::loop_::Timeout::Finite(Duration::ZERO));
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| err.to_string())?;
        Ok(Pipewire {
            core,
            _context: context,
            handle: handle.clone(),
            token,
        })
    }
}

impl Drop for Pipewire {
    fn drop(&mut self) {
        self.handle.remove(self.token);
    }
}

pub(super) struct Feed {
    // Unhooked before the stream goes: the other way round it writes into the freed stream.
    _listener: pw::stream::StreamListener<()>,
    stream: pw::stream::StreamRc,
    streaming: Rc<Cell<bool>>,
    /// Shared with the listener, which sizes buffers by it once a format is agreed.
    size: Rc<Cell<(u32, u32)>>,
    /// The timer for the next frame, while one is due.
    tick: Option<RegistrationToken>,
    /// This process and a count, so a restarted daemon's streams never look like the old ones.
    serial: u64,
}

static STREAMS: AtomicU32 = AtomicU32::new(0);

/// A monitor's stream started or stopped being watched, or broke.
pub(super) struct Change {
    screen: u64,
    streaming: bool,
    failed: bool,
}

impl Feed {
    fn new(
        core: &pw::core::CoreRc,
        screen: u64,
        name: &str,
        size: (u32, u32),
        changes: Sender<Change>,
    ) -> Result<Self, String> {
        let failed = |err: pw::Error| format!("PipeWire stream: {err}");
        let mut properties = pw::properties::PropertiesBox::new();
        properties.insert(*pw::keys::MEDIA_TYPE, "Video");
        properties.insert(*pw::keys::MEDIA_CLASS, "Video/Source");
        properties.insert(*pw::keys::NODE_NAME, format!("apolline-wall.{name}"));
        properties.insert(
            *pw::keys::NODE_DESCRIPTION,
            format!("apolline-wall wallpaper for {name}"),
        );
        let stream =
            pw::stream::StreamRc::new(core.clone(), "apolline-wall", properties).map_err(failed)?;
        let streaming = Rc::new(Cell::new(false));
        let seen = Rc::clone(&streaming);
        let size = Rc::new(Cell::new(size));
        let wanted = Rc::clone(&size);
        let listener = stream
            .add_local_listener_with_user_data(())
            .state_changed(move |_, _, _, state| {
                let now = matches!(state, StreamState::Streaming);
                let broke = matches!(state, StreamState::Error(_));
                if now != seen.get() || broke {
                    seen.set(now);
                    let _ = changes.send(Change {
                        screen,
                        streaming: now,
                        failed: broke,
                    });
                }
            })
            .param_changed(move |stream, _, id, param| {
                if id != ParamType::Format.as_raw() || param.is_none() {
                    return;
                }
                if let Some(bytes) = pod_bytes(buffers(wanted.get()))
                    && let Some(pod) = Pod::from_bytes(&bytes)
                {
                    let _ = stream.update_params(&mut [pod]);
                }
            })
            .register()
            .map_err(failed)?;
        let format = format_bytes(size.get())?;
        let format = Pod::from_bytes(&format).ok_or("PipeWire: the stream format didn't encode")?;
        // PipeWire allocates the shared memory the Buffers param asks for and maps it.
        stream
            .connect(
                spa::utils::Direction::Output,
                None,
                StreamFlags::DRIVER | StreamFlags::MAP_BUFFERS,
                &mut [format],
            )
            .map_err(failed)?;
        Ok(Feed {
            _listener: listener,
            stream,
            streaming,
            size,
            tick: None,
            serial: u64::from(std::process::id()) << 32
                | u64::from(STREAMS.fetch_add(1, Ordering::Relaxed)),
        })
    }

    /// The same stream at a new size: the plugin renegotiates and stays on the same node,
    /// where a new stream could get the old one's id and be missed.
    fn resize(&self, size: (u32, u32)) -> Result<(), String> {
        self.size.set(size);
        let format = format_bytes(size)?;
        let format = Pod::from_bytes(&format).ok_or("PipeWire: the stream format didn't encode")?;
        self.stream
            .update_params(&mut [format])
            .map_err(|err| format!("PipeWire stream: {err}"))
    }

    /// The stream's node once PipeWire has made it.
    fn described(&self) -> Option<FeedStream> {
        let node = self.stream.node_id();
        (node != pw::constants::ID_ANY).then_some(FeedStream {
            node,
            stream: self.serial,
        })
    }

    /// Hands the frame just drawn to whoever watches. One that finds every buffer still
    /// with the plugin is dropped: the next will do.
    fn push(&self, gl: &Gl, target: &Target) -> Result<(), String> {
        let (w, h) = self.size.get();
        if !self.streaming.get() || target.size() != (w, h) {
            return Ok(());
        }
        let Some(mut buffer) = self.stream.dequeue_buffer() else {
            return Ok(());
        };
        let Some(data) = buffer.datas_mut().first_mut() else {
            return Ok(());
        };
        let stride = w * 4;
        let bytes = stride as usize * h as usize;
        let read = match data.data().and_then(|memory| memory.get_mut(..bytes)) {
            Some(frame) => gl.read_frame(target, frame),
            None => Err("a PipeWire buffer too small for the frame".into()),
        };
        // The buffer goes back either way; an empty one tells the plugin there's nothing new.
        let chunk = data.chunk_mut();
        *chunk.offset_mut() = 0;
        *chunk.stride_mut() = stride as i32;
        *chunk.size_mut() = if read.is_ok() { bytes as u32 } else { 0 };
        drop(buffer);
        read?;
        if self.stream.is_driving() {
            self.stream
                .trigger_process()
                .map_err(|err| format!("PipeWire stream: {err}"))?;
        }
        Ok(())
    }
}

/// One fixed format: the monitor's own pixels, RGB with the fourth byte unused, frames when
/// they come.
fn format((w, h): (u32, u32)) -> Object {
    Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: vec![
            Property::new(
                FormatProperties::MediaType.as_raw(),
                Value::Id(Id(MediaType::Video.as_raw())),
            ),
            Property::new(
                FormatProperties::MediaSubtype.as_raw(),
                Value::Id(Id(MediaSubtype::Raw.as_raw())),
            ),
            Property::new(
                FormatProperties::VideoFormat.as_raw(),
                Value::Id(Id(VideoFormat::RGBx.as_raw())),
            ),
            Property::new(
                FormatProperties::VideoSize.as_raw(),
                Value::Rectangle(Rectangle {
                    width: w,
                    height: h,
                }),
            ),
            Property::new(
                FormatProperties::VideoFramerate.as_raw(),
                Value::Fraction(Fraction { num: 0, denom: 1 }),
            ),
        ],
    }
}

/// Shared-memory buffers PipeWire allocates, a frame each.
fn buffers((w, h): (u32, u32)) -> Object {
    let stride = (w * 4) as i32;
    Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: ParamType::Buffers.as_raw(),
        properties: vec![
            Property::new(
                spa::sys::SPA_PARAM_BUFFERS_buffers,
                Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 4,
                        min: 2,
                        max: 8,
                    },
                ))),
            ),
            Property::new(spa::sys::SPA_PARAM_BUFFERS_blocks, Value::Int(1)),
            Property::new(
                spa::sys::SPA_PARAM_BUFFERS_size,
                Value::Int(stride * h as i32),
            ),
            Property::new(spa::sys::SPA_PARAM_BUFFERS_stride, Value::Int(stride)),
            Property::new(
                spa::sys::SPA_PARAM_BUFFERS_dataType,
                Value::Int(1 << spa::buffer::DataType::MemFd.as_raw()),
            ),
        ],
    }
}

fn format_bytes(size: (u32, u32)) -> Result<Vec<u8>, String> {
    pod_bytes(format(size)).ok_or_else(|| "PipeWire: the stream format didn't encode".into())
}

fn pod_bytes(object: Object) -> Option<Vec<u8>> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )
    .ok()
    .map(|(cursor, _)| cursor.into_inner())
}

impl App {
    /// The plugin asking for `output`'s frames, and saying whether windows hide the desktop
    /// there. It asks again now and then, to find the node after the daemon restarts.
    pub(super) fn feed(&mut self, output: &str, covered: Option<bool>) {
        let Some(index) = self.screens.iter().position(|screen| screen.name == output) else {
            return feed_error(output, format!("there is no monitor {output}"));
        };
        if self.screens[index].feed.is_none()
            && let Err(message) = self.start_feed(index)
        {
            return feed_error(output, message);
        }
        if let Some(covered) = covered
            && self.screens[index].covered != covered
        {
            self.screens[index].covered = covered;
            self.sync_policy();
            protocol::emit(&self.status());
        }
    }

    fn start_feed(&mut self, index: usize) -> Result<(), String> {
        if !matches!(self.desktop, Desktop::Wayland(_)) {
            return Err("Plasma's wallpaper plugin needs a Wayland session".into());
        }
        let size = self
            .buffer_size(index)
            .ok_or("the monitor has no size yet")?;
        let feed = self.new_feed(index, size)?;
        self.release_target(index);
        self.wayland_hide(index);
        let screen = &mut self.screens[index];
        screen.feed = Some(feed);
        screen.frame_pending = false;
        self.place(index);
        Ok(())
    }

    fn new_feed(&mut self, index: usize, size: (u32, u32)) -> Result<Feed, String> {
        if self.pipewire.is_none() {
            self.pipewire = Some(Pipewire::connect(&self.handle)?);
        }
        let core = &self.pipewire.as_ref().expect("connected above").core;
        let screen = &self.screens[index];
        Feed::new(
            core,
            screen.id,
            &screen.name,
            size,
            self.feed_changes.clone(),
        )
    }

    /// Back to the monitor's own surface, which draws from its first configure. Only for a
    /// stream that broke: the plugin asks again and gets a new one.
    fn stop_feed(&mut self, index: usize) {
        let Some(feed) = self.screens[index].feed.take() else {
            return;
        };
        if let Some(tick) = feed.tick {
            self.handle.remove(tick);
        }
        drop(feed);
        let covered = std::mem::take(&mut self.screens[index].covered);
        self.release_target(index);
        self.screens[index].frame_pending = false;
        if !self.screens[index].hidden {
            self.wayland_reopen(index);
        }
        if covered {
            self.sync_policy();
        }
    }

    pub(super) fn feed_changed(&mut self, change: Change) {
        let Some(index) = self.index_of(change.screen) else {
            return;
        };
        let Some(feed) = &self.screens[index].feed else {
            return;
        };
        let ticking = feed.tick.is_some();
        if change.failed {
            let name = self.screens[index].name.clone();
            self.stop_feed(index);
            // PipeWire itself may be gone; the next ask connects afresh.
            if self.screens.iter().all(|screen| screen.feed.is_none()) {
                self.pipewire = None;
            }
            return feed_error(&name, "its PipeWire stream failed".into());
        }
        // Back from a pause with a frame already due, the clock carries on by itself.
        if !change.streaming || ticking {
            return;
        }
        if self.screens[index].frame_pending {
            self.frame_done(index);
        } else {
            self.draw(index);
        }
    }

    /// The next frame, on a clock of our own; none while nobody watches.
    pub(super) fn feed_frame(&mut self, index: usize) {
        let id = self.screens[index].id;
        let Some(feed) = &mut self.screens[index].feed else {
            return;
        };
        if !feed.streaming.get() || feed.tick.is_some() {
            return;
        }
        feed.tick = self
            .handle
            .insert_source(Timer::from_duration(FRAME), move |_, _, app: &mut App| {
                if let Some(index) = app.index_of(id) {
                    if let Some(feed) = &mut app.screens[index].feed {
                        feed.tick = None;
                    }
                    app.frame_done(index);
                }
                TimeoutAction::Drop
            })
            .ok();
    }

    /// An offscreen target the monitor's size, the stream following it.
    pub(super) fn place_feed(&mut self, index: usize, size: (u32, u32)) {
        let resized = match &self.screens[index].feed {
            Some(feed) if feed.size.get() != size => feed.resize(size),
            _ => Ok(()),
        };
        if let Err(message) = resized {
            let name = self.screens[index].name.clone();
            self.stop_feed(index);
            return feed_error(&name, message);
        }
        // linux-wallpaperengine draws this monitor, which the plugin can't show anyway.
        if self.screens[index].hidden {
            return;
        }
        let screen = &mut self.screens[index];
        let placed = match &mut screen.target {
            Some(target) if target.size() == size => Ok(()),
            Some(target) => self.gl.resize(target, size),
            None => self
                .gl
                .offscreen(size)
                .map(|target| screen.target = Some(target)),
        };
        if let Err(message) = placed {
            return feed_error(&screen.name.clone(), message);
        }
        self.draw(index);
        self.refresh();
    }

    pub(super) fn push_feed(&self, index: usize) -> Result<(), String> {
        let screen = &self.screens[index];
        match (&screen.feed, &screen.target) {
            (Some(feed), Some(target)) => feed.push(&self.gl, target),
            _ => Ok(()),
        }
    }

    pub(super) fn feed_stream(&self, index: usize) -> Option<FeedStream> {
        self.screens[index].feed.as_ref().and_then(Feed::described)
    }
}

fn feed_error(output: &str, message: String) {
    protocol::emit(&Event::Error {
        output: Some(output.to_owned()),
        source: None,
        message: format!("Plasma's wallpaper plugin: {message}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pw::spa::pod::deserialize::PodDeserializer;

    fn decoded(object: Object) -> Vec<Property> {
        let bytes = pod_bytes(object).expect("encodes");
        match PodDeserializer::deserialize_any_from(&bytes)
            .expect("decodes")
            .1
        {
            Value::Object(object) => object.properties,
            other => panic!("not an object: {other:?}"),
        }
    }

    fn value(properties: &[Property], key: u32) -> &Value {
        &properties
            .iter()
            .find(|property| property.key == key)
            .expect("the property is there")
            .value
    }

    #[test]
    fn the_stream_carries_the_monitors_own_pixels() {
        let format = decoded(format((2560, 1440)));
        assert_eq!(
            value(&format, FormatProperties::VideoSize.as_raw()),
            &Value::Rectangle(Rectangle {
                width: 2560,
                height: 1440
            })
        );
        assert_eq!(
            value(&format, FormatProperties::VideoFormat.as_raw()),
            &Value::Id(Id(VideoFormat::RGBx.as_raw()))
        );
    }

    #[test]
    fn a_buffer_holds_one_frame_in_shared_memory() {
        let buffers = decoded(buffers((2560, 1440)));
        assert_eq!(
            value(&buffers, spa::sys::SPA_PARAM_BUFFERS_stride),
            &Value::Int(2560 * 4)
        );
        assert_eq!(
            value(&buffers, spa::sys::SPA_PARAM_BUFFERS_size),
            &Value::Int(2560 * 4 * 1440)
        );
        assert_eq!(
            value(&buffers, spa::sys::SPA_PARAM_BUFFERS_dataType),
            &Value::Int(1 << spa::buffer::DataType::MemFd.as_raw())
        );
    }
}
