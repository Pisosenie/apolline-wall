use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{self, Sender};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::wl_output::{Transform, WlOutput};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::{
    self, WpViewport,
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState, SimpleGlobal};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_simple,
    registry_handlers,
};

use crate::gl::{Gl, Target, Texture};
use crate::picture::{self, Pixels};
use crate::protocol::{self, Command, Event, Fit, Show};

pub fn run() -> Result<(), String> {
    share_arenas();
    let conn = Connection::connect_to_env().map_err(|err| format!("Wayland: {err}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|err| err.to_string())?;
    let qh = queue.handle();
    let mut event_loop = EventLoop::<App>::try_new().map_err(|err| err.to_string())?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|err| err.to_string())?;

    let (commands, incoming) = channel::channel();
    std::thread::spawn(move || read_commands(commands));
    handle
        .insert_source(incoming, |event, _, app: &mut App| match event {
            channel::Event::Msg(Ok(command)) => app.command(command),
            channel::Event::Msg(Err(message)) => protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            }),
            channel::Event::Closed => app.exit = true,
        })
        .map_err(|err| err.to_string())?;

    let (loads, loaded) = channel::channel();
    handle
        .insert_source(loaded, |event, _, app: &mut App| {
            if let channel::Event::Msg(loaded) = event {
                app.loaded(loaded);
            }
        })
        .map_err(|err| err.to_string())?;
    let (decoder, batches) = mpsc::channel();
    std::thread::spawn(move || decode(batches, loads));

    let mut app = App {
        screens: Vec::new(),
        gl: Gl::new(conn.backend().display_ptr().cast())?,
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)
            .map_err(|err| format!("wl_compositor: {err}"))?,
        layers: LayerShell::bind(&globals, &qh).map_err(|err| format!("layer shell: {err}"))?,
        viewporter: SimpleGlobal::bind(&globals, &qh).ok(),
        qh,
        everywhere: None,
        per_output: HashMap::new(),
        paused_everywhere: false,
        paused: HashSet::new(),
        decoder,
        generation: 0,
        exit: false,
    };
    while !app.exit {
        event_loop
            .dispatch(None, &mut app)
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn read_commands(commands: Sender<Result<Command, String>>) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !line.trim().is_empty() && commands.send(protocol::parse(&line)).is_err() {
            break;
        }
    }
}

struct App {
    screens: Vec<Screen>,
    gl: Gl,
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layers: LayerShell,
    viewporter: Option<SimpleGlobal<WpViewporter, 1>>,
    qh: QueueHandle<App>,
    everywhere: Option<Show>,
    per_output: HashMap<String, Show>,
    paused_everywhere: bool,
    paused: HashSet<String>,
    decoder: mpsc::Sender<Batch>,
    generation: u64,
    exit: bool,
}

struct Screen {
    target: Option<Target>,
    output: WlOutput,
    name: String,
    layer: LayerSurface,
    viewport: Option<WpViewport>,
    logical: Option<(u32, u32)>,
    shown: Option<Shown>,
    fade: Option<Fade>,
    loading: Option<Loading>,
    frame_pending: bool,
}

struct Shown {
    texture: Texture,
    show: Show,
    size: (u32, u32),
}

struct Fade {
    before: Texture,
    start: Instant,
    duration: Duration,
}

struct Loading {
    generation: u64,
    show: Show,
    size: (u32, u32),
}

struct Batch {
    source: PathBuf,
    fit: Fit,
    jobs: Vec<Job>,
}

struct Job {
    output: WlOutput,
    generation: u64,
    size: (u32, u32),
}

struct Loaded {
    output: WlOutput,
    generation: u64,
    source: PathBuf,
    result: Result<Pixels, String>,
}

impl App {
    fn command(&mut self, command: Command) {
        match command {
            Command::Show(show) => {
                match &show.output {
                    None => {
                        self.per_output.clear();
                        self.everywhere = Some(show);
                    }
                    Some(name) => {
                        self.per_output.insert(name.clone(), show);
                    }
                }
                self.refresh();
            }
            Command::Pause(pause) => {
                match pause.output {
                    None => {
                        self.paused_everywhere = pause.paused;
                        self.paused.clear();
                    }
                    Some(name) if pause.paused => {
                        self.paused.insert(name);
                    }
                    Some(name) => {
                        self.paused.remove(&name);
                    }
                }
                for index in 0..self.screens.len() {
                    if self.screens[index].fade.is_some() {
                        self.draw(index);
                    }
                }
            }
        }
    }

    fn is_paused(&self, name: &str) -> bool {
        self.paused_everywhere || self.paused.contains(name)
    }

    fn refresh(&mut self) {
        let mut jobs: HashMap<(PathBuf, Fit), Vec<Job>> = HashMap::new();
        for index in 0..self.screens.len() {
            let Some(size) = self.buffer_size(index) else {
                continue;
            };
            let screen = &self.screens[index];
            let Some(show) = self
                .per_output
                .get(&screen.name)
                .or(self.everywhere.as_ref())
                .cloned()
            else {
                continue;
            };
            let latest = match (&screen.loading, &screen.shown) {
                (Some(loading), _) => Some((&loading.show, loading.size)),
                (None, Some(shown)) => Some((&shown.show, shown.size)),
                (None, None) => None,
            };
            if latest.is_some_and(|(latest, latest_size)| {
                latest.source == show.source && latest.fit == show.fit && latest_size == size
            }) {
                continue;
            }
            self.generation += 1;
            jobs.entry((show.source.clone(), show.fit))
                .or_default()
                .push(Job {
                    output: screen.output.clone(),
                    generation: self.generation,
                    size,
                });
            self.screens[index].loading = Some(Loading {
                generation: self.generation,
                show,
                size,
            });
        }
        for ((source, fit), jobs) in jobs {
            let _ = self.decoder.send(Batch { source, fit, jobs });
        }
    }

    fn loaded(&mut self, loaded: Loaded) {
        let Some(index) = self.screens.iter().position(|s| s.output == loaded.output) else {
            return;
        };
        let paused = self.is_paused(&self.screens[index].name);
        let screen = &mut self.screens[index];
        if screen.loading.as_ref().map(|l| l.generation) != Some(loaded.generation) {
            return;
        }
        let Some(loading) = screen.loading.take() else {
            return;
        };
        let texture = loaded.result.and_then(|pixels| match &screen.target {
            Some(target) => self.gl.upload(target, &pixels),
            None => Err("the output has no surface yet".into()),
        });
        let texture = match texture {
            Ok(texture) => texture,
            Err(message) => {
                protocol::emit(&Event::Error {
                    output: Some(screen.name.clone()),
                    source: Some(loaded.source),
                    message,
                });
                return;
            }
        };
        give_back_memory();
        if let Some(fade) = screen.fade.take() {
            self.gl.delete(fade.before);
        }
        let duration = Duration::from_millis(loading.show.transition_ms.into());
        match screen.shown.take() {
            Some(old) if !paused && !duration.is_zero() => {
                screen.fade = Some(Fade {
                    before: old.texture,
                    start: Instant::now(),
                    duration,
                })
            }
            Some(old) => self.gl.delete(old.texture),
            None => {}
        }
        let event = Event::Ready {
            output: screen.name.clone(),
            source: loading.show.source.clone(),
            poster: loading.show.source.clone(),
        };
        screen.shown = Some(Shown {
            texture,
            show: loading.show,
            size: loading.size,
        });
        self.draw(index);
        protocol::emit(&event);
    }

    fn draw(&mut self, index: usize) {
        let paused = self.is_paused(&self.screens[index].name);
        let screen = &mut self.screens[index];
        let (Some(target), Some(shown)) = (&screen.target, &screen.shown) else {
            return;
        };
        let progress = match &screen.fade {
            Some(fade) if !paused => {
                fade.start.elapsed().as_secs_f32() / fade.duration.as_secs_f32()
            }
            _ => 1.0,
        };
        let fading = progress < 1.0;
        if fading && !screen.frame_pending {
            let surface = screen.layer.wl_surface();
            surface.frame(&self.qh, surface.clone());
            screen.frame_pending = true;
        }
        let before = screen
            .fade
            .as_ref()
            .filter(|_| fading)
            .map(|fade| fade.before);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        if let Err(message) = self.gl.draw(target, before, shown.texture, eased.min(1.0)) {
            protocol::emit(&Event::Error {
                output: Some(screen.name.clone()),
                source: None,
                message,
            });
        }
        if !fading && let Some(fade) = screen.fade.take() {
            self.gl.delete(fade.before);
        }
    }

    fn buffer_size(&self, index: usize) -> Option<(u32, u32)> {
        let screen = &self.screens[index];
        let (w, h) = screen.logical?;
        let info = self.outputs.info(&screen.output)?;
        let scale = info.scale_factor.max(1) as u32;
        let physical = self.viewporter.as_ref().and_then(|_| current_mode(&info));
        Some(physical.unwrap_or((w * scale, h * scale)))
    }

    fn place(&mut self, index: usize) {
        let Some(size) = self.buffer_size(index) else {
            return;
        };
        let scale = self
            .outputs
            .info(&self.screens[index].output)
            .map_or(1, |info| info.scale_factor.max(1));
        let screen = &mut self.screens[index];
        let Some((w, h)) = screen.logical else {
            return;
        };
        let surface = screen.layer.wl_surface();
        match &screen.viewport {
            Some(viewport) => viewport.set_destination(w as i32, h as i32),
            None => surface.set_buffer_scale(scale),
        }
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, 0, w as i32, h as i32);
            surface.set_opaque_region(Some(region.wl_region()));
        }
        let placed = match &mut screen.target {
            Some(target) if target.size() == size => Ok(()),
            Some(target) => self.gl.resize(target, size),
            None => self
                .gl
                .target(surface.id().as_ptr().cast(), size)
                .map(|target| screen.target = Some(target)),
        };
        if let Err(message) = placed {
            protocol::emit(&Event::Error {
                output: Some(screen.name.clone()),
                source: None,
                message,
            });
            return;
        }
        self.draw(index);
        self.refresh();
    }

    fn forget(&mut self, index: usize) {
        let mut screen = self.screens.remove(index);
        drop(screen.target.take());
        for texture in screen
            .shown
            .map(|s| s.texture)
            .into_iter()
            .chain(screen.fade.map(|f| f.before))
        {
            self.gl.delete(texture);
        }
        if let Some(viewport) = screen.viewport {
            viewport.destroy();
        }
    }

    fn screen_of(&self, surface: &WlSurface) -> Option<usize> {
        self.screens
            .iter()
            .position(|screen| screen.layer.wl_surface() == surface)
    }
}

fn decode(batches: mpsc::Receiver<Batch>, loads: Sender<Loaded>) {
    for batch in batches {
        let decoded = picture::decode(&batch.source);
        for job in batch.jobs {
            let (width, height) = job.size;
            let result = decoded
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|image| picture::fit(image, width, height, batch.fit));
            let loaded = Loaded {
                output: job.output,
                generation: job.generation,
                source: batch.source.clone(),
                result,
            };
            if loads.send(loaded).is_err() {
                return;
            }
        }
        drop(decoded);
        give_back_memory();
    }
}

// Decoded images run to tens of megabytes, and glibc keeps freed ones around unless asked;
// a wallpaper sits idle almost always, so a couple of shared arenas beat one per GL driver thread.
#[cfg(target_env = "gnu")]
fn give_back_memory() {
    unsafe { libc::malloc_trim(0) };
}

#[cfg(target_env = "gnu")]
fn share_arenas() {
    unsafe { libc::mallopt(libc::M_ARENA_MAX, 2) };
}

#[cfg(not(target_env = "gnu"))]
fn give_back_memory() {}

#[cfg(not(target_env = "gnu"))]
fn share_arenas() {}

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
            self.screens[index].frame_pending = false;
            if self.screens[index].fade.is_some() {
                self.draw(index);
            }
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
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {
        let name = self
            .outputs
            .info(&output)
            .and_then(|info| info.name)
            .unwrap_or_else(|| format!("output-{}", output.id().protocol_id()));
        let surface = self.compositor.create_surface(qh);
        let viewport = self
            .viewporter
            .as_ref()
            .and_then(|viewporter| viewporter.get().ok())
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        let layer = self.layers.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some("apolline-wallpaper"),
            Some(&output),
        );
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.screens.push(Screen {
            target: None,
            output,
            name,
            layer,
            viewport,
            logical: None,
            shown: None,
            fade: None,
            loading: None,
            frame_pending: false,
        });
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        if let Some(index) = self.screens.iter().position(|s| s.output == output) {
            self.place(index);
        }
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        if let Some(index) = self.screens.iter().position(|s| s.output == output) {
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
            self.screens[index].logical = Some((w, h));
            self.place(index);
        }
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState];
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
delegate_registry!(App);
delegate_simple!(App, WpViewporter, 1);
