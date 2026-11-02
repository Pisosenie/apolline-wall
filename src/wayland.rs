mod playback;
mod shading;
mod socket;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io::BufRead;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Sender};
use smithay_client_toolkit::reexports::calloop::ping::{self, Ping};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};
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

use apolline_wall_protocol::source::{self, Kind, poster_path};

use crate::gl::{self, Canvas, Gl, Target, Texture};
use crate::picture::{self, Pixels, UPSIDE_DOWN, WHOLE};
use crate::protocol::{self, Command, Event, Fit, Monitor, Show};
use crate::state::{self, State};
use crate::video::{Mpv, Player};

/// Apolline starts it as a child and talks over stdin; on its own it runs as a daemon and remembers.
pub enum Mode {
    Child,
    Daemon,
}

pub fn run(mode: Mode) -> Result<(), String> {
    share_arenas();
    shading::local_time_zone();
    let conn = Connection::connect_to_env().map_err(|err| format!("Wayland: {err}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|err| err.to_string())?;
    let qh = queue.handle();
    let mut event_loop = EventLoop::<App>::try_new().map_err(|err| err.to_string())?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|err| err.to_string())?;
    socket::serve(&handle)?;

    if let Mode::Child = mode {
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
    }

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

    let (wake, woken) = ping::make_ping().map_err(|err| err.to_string())?;
    handle
        .insert_source(woken, |_, _, app: &mut App| app.tick())
        .map_err(|err| err.to_string())?;

    let wl_display: *mut c_void = conn.backend().display_ptr().cast();
    let mut app = App {
        screens: Vec::new(),
        videos: HashMap::new(),
        gl: Gl::new(wl_display)?,
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
        mpv: None,
        wake,
        wl_display,
        handle,
        fps: shading::DEFAULT_FPS,
        scale: None,
        accent: shading::DEFAULT_ACCENT,
        persist: None,
        generation: 0,
        exit: false,
    };
    if let Mode::Daemon = mode {
        app.persist = restore(&mut app);
    }
    while !app.exit {
        event_loop
            .dispatch(None, &mut app)
            .map_err(|err| err.to_string())?;
    }
    app.close_videos();
    let _ = std::fs::remove_file(protocol::socket_path());
    Ok(())
}

/// Replays the last session; a state file that doesn't parse is reported and left alone.
fn restore(app: &mut App) -> Option<Persist> {
    let path = state::path()?;
    match state::load(&path) {
        Ok(saved) => {
            for command in saved.commands() {
                app.command(command);
            }
            Some(Persist {
                path,
                good: saved.clone(),
                state: saved,
            })
        }
        Err(message) => {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message: format!("{message}; not saving until it's fixed"),
            });
            None
        }
    }
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
    videos: HashMap<PathBuf, Video>,
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
    mpv: Option<Rc<Mpv>>,
    wake: Ping,
    wl_display: *mut c_void,
    handle: LoopHandle<'static, App>,
    fps: u32,
    scale: Option<f32>,
    accent: [f32; 3],
    persist: Option<Persist>,
    generation: u64,
    exit: bool,
}

/// The daemon's memory: what to restore, and what last showed for when a wallpaper fails.
struct Persist {
    path: PathBuf,
    state: State,
    good: State,
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
    stale: bool,
    failed: Option<Show>,
}

enum Picture {
    Still(Texture),
    Video(PathBuf),
    Shader(Box<Scene>),
}

struct Shown {
    picture: Picture,
    show: Show,
    size: (u32, u32),
}

struct Fade {
    before: gl::Layer,
    start: Instant,
    duration: Duration,
}

struct Loading {
    generation: u64,
    show: Show,
    size: (u32, u32),
}

/// One player per file, whatever the number of monitors showing it.
struct Video {
    player: Player,
    canvas: Option<Canvas>,
    size: Option<(u32, u32)>,
    started: bool,
    waiting: bool,
    paused: bool,
    poster: Option<PathBuf>,
    unannounced: Vec<String>,
}

/// A shader drawn into its own canvas for one monitor, at a capped rate.
struct Scene {
    shader: gl::Shader,
    canvas: Canvas,
    time: f32,
    last: Option<Instant>,
    frame: i32,
    timer: bool,
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
        if let Some(persist) = &mut self.persist
            && persist.state.record(&command)
            && let Err(message) = state::save(&persist.path, &persist.state)
        {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            });
        }
        match command {
            Command::Show(show) => {
                for screen in &mut self.screens {
                    if show.output.as_ref().is_none_or(|name| *name == screen.name) {
                        screen.failed = None;
                    }
                }
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
                self.sync_pauses();
                self.pause_scenes();
                for index in 0..self.screens.len() {
                    if self.screens[index].fade.is_some() {
                        self.draw(index);
                    }
                }
            }
            Command::Limits(limits) => self.limit(limits),
            Command::Uniforms(uniforms) => self.tint(uniforms),
            Command::Status {} => protocol::emit(&self.status()),
        }
    }

    fn status(&self) -> Event {
        Event::Status {
            outputs: self
                .screens
                .iter()
                .map(|screen| Monitor {
                    name: screen.name.clone(),
                    source: screen.shown.as_ref().map(|shown| shown.show.source.clone()),
                    paused: self.is_paused(&screen.name),
                })
                .collect(),
            fps: self.fps,
            scale: self.scale,
        }
    }

    fn is_paused(&self, name: &str) -> bool {
        self.paused_everywhere || self.paused.contains(name)
    }

    fn wanted(&self, index: usize) -> Option<Show> {
        let name = &self.screens[index].name;
        self.per_output
            .get(name)
            .or(self.everywhere.as_ref())
            .cloned()
    }

    fn refresh(&mut self) {
        let mut jobs: HashMap<(PathBuf, Fit), Vec<Job>> = HashMap::new();
        let mut videos = Vec::new();
        let mut scenes = Vec::new();
        for index in 0..self.screens.len() {
            let (Some(size), Some(show)) = (self.buffer_size(index), self.wanted(index)) else {
                continue;
            };
            let kind = source::kind(&show.source);
            let screen = &self.screens[index];
            if screen.failed.as_ref() == Some(&show) {
                continue;
            }
            let latest = match (&screen.loading, &screen.shown) {
                (Some(loading), _) => Some((&loading.show, loading.size)),
                (None, Some(shown)) => Some((&shown.show, shown.size)),
                (None, None) => None,
            };
            if latest.is_some_and(|(latest, latest_size)| {
                latest.source == show.source
                    && latest.fit == show.fit
                    && (kind == Kind::Video || latest_size == size)
            }) {
                continue;
            }
            self.generation += 1;
            match kind {
                Kind::Video => videos.push(index),
                Kind::Shader => scenes.push(index),
                Kind::Picture => jobs
                    .entry((show.source.clone(), show.fit))
                    .or_default()
                    .push(Job {
                        output: screen.output.clone(),
                        generation: self.generation,
                        size,
                    }),
            }
            self.screens[index].loading = Some(Loading {
                generation: self.generation,
                show,
                size,
            });
        }
        for ((source, fit), jobs) in jobs {
            let _ = self.decoder.send(Batch { source, fit, jobs });
        }
        for index in videos {
            self.start_video(index);
        }
        for index in scenes {
            self.start_scene(index);
        }
        self.fit_canvases();
    }

    fn loaded(&mut self, loaded: Loaded) {
        let Some(index) = self.screens.iter().position(|s| s.output == loaded.output) else {
            return;
        };
        let screen = &mut self.screens[index];
        if screen.loading.as_ref().map(|l| l.generation) != Some(loaded.generation) {
            return;
        }
        let Some(loading) = screen.loading.take() else {
            return;
        };
        let texture = loaded.result.and_then(|pixels| self.gl.upload(&pixels));
        give_back_memory();
        let output = self.screens[index].name.clone();
        match texture {
            Ok(texture) => {
                let source = loading.show.source.clone();
                self.present(index, Picture::Still(texture), loading);
                protocol::emit(&Event::Ready {
                    output,
                    poster: source.clone(),
                    source,
                });
            }
            Err(message) => {
                self.give_up(index, loading.show);
                protocol::emit(&Event::Error {
                    output: Some(output),
                    source: Some(loaded.source),
                    message,
                });
            }
        }
    }

    fn give_up(&mut self, index: usize, show: Show) {
        if let Some(persist) = &mut self.persist
            && persist.state.undo(&show, &persist.good)
            && let Err(message) = state::save(&persist.path, &persist.state)
        {
            protocol::emit(&Event::Error {
                output: None,
                source: None,
                message,
            });
        }
        self.screens[index].failed = Some(show);
    }

    fn present(&mut self, index: usize, picture: Picture, loading: Loading) {
        if let Some(persist) = &mut self.persist {
            persist.good.keep(&loading.show);
        }
        let paused = self.is_paused(&self.screens[index].name);
        let before = self.screens[index]
            .shown
            .take()
            .and_then(|old| self.retire(index, old));
        let screen = &mut self.screens[index];
        if let Some(fade) = screen.fade.take() {
            self.gl.delete(fade.before.texture);
        }
        let duration = Duration::from_millis(loading.show.transition_ms.into());
        match before {
            Some(before) if !paused && !duration.is_zero() => {
                screen.fade = Some(Fade {
                    before,
                    start: Instant::now(),
                    duration,
                })
            }
            Some(before) => self.gl.delete(before.texture),
            None => {}
        }
        screen.shown = Some(Shown {
            picture,
            show: loading.show,
            size: loading.size,
        });
        self.draw(index);
        self.close_unused_videos();
        self.sync_pauses();
    }

    fn retire(&self, index: usize, old: Shown) -> Option<gl::Layer> {
        match old.picture {
            Picture::Still(texture) => Some(gl::Layer { texture, uv: WHOLE }),
            Picture::Video(source) => {
                let canvas = self.videos.get(&source)?.canvas.as_ref()?;
                let target = self.screens[index].target.as_ref()?;
                Some(gl::Layer {
                    texture: self.gl.snapshot(canvas).ok()?,
                    uv: picture::placement(old.show.fit, canvas.size, target.size()),
                })
            }
            Picture::Shader(scene) => {
                let texture = self.gl.snapshot(&scene.canvas).ok();
                self.drop_scene(*scene);
                Some(gl::Layer {
                    texture: texture?,
                    uv: UPSIDE_DOWN,
                })
            }
        }
    }

    fn draw(&mut self, index: usize) {
        let paused = self.is_paused(&self.screens[index].name);
        let screen = &mut self.screens[index];
        let (Some(target), Some(shown)) = (&screen.target, &screen.shown) else {
            return;
        };
        let after = match &shown.picture {
            Picture::Still(texture) => gl::Layer {
                texture: *texture,
                uv: WHOLE,
            },
            Picture::Video(source) => {
                let Some(canvas) = self.videos.get(source).and_then(|v| v.canvas.as_ref()) else {
                    return;
                };
                gl::Layer {
                    texture: canvas.texture,
                    uv: picture::placement(shown.show.fit, canvas.size, target.size()),
                }
            }
            Picture::Shader(scene) => gl::Layer {
                texture: scene.canvas.texture,
                uv: UPSIDE_DOWN,
            },
        };
        let progress = match &screen.fade {
            Some(fade) if !paused => {
                fade.start.elapsed().as_secs_f32() / fade.duration.as_secs_f32()
            }
            _ => 1.0,
        };
        let fading = progress < 1.0;
        let playing = matches!(shown.picture, Picture::Video(_) | Picture::Shader(_)) && !paused;
        if (fading || playing) && !screen.frame_pending {
            let surface = screen.layer.wl_surface();
            surface.frame(&self.qh, surface.clone());
            screen.frame_pending = true;
        }
        screen.stale = false;
        let before = screen
            .fade
            .as_ref()
            .filter(|_| fading)
            .map(|fade| fade.before);
        let eased = progress * progress * (3.0 - 2.0 * progress);
        if let Err(message) = self.gl.draw(target, before, after, eased.min(1.0)) {
            protocol::emit(&Event::Error {
                output: Some(screen.name.clone()),
                source: None,
                message,
            });
        }
        if !fading && let Some(fade) = screen.fade.take() {
            self.gl.delete(fade.before.texture);
        }
    }

    fn frame_done(&mut self, index: usize) {
        self.screens[index].frame_pending = false;
        if let Some(Picture::Video(source)) = self.screens[index].shown.as_ref().map(|s| &s.picture)
        {
            let source = source.clone();
            if self.videos.get(&source).is_some_and(|video| video.waiting) {
                self.pump(&source);
            }
        }
        let shader = matches!(
            self.screens[index].shown.as_ref().map(|s| &s.picture),
            Some(Picture::Shader(_))
        );
        if shader && !self.is_paused(&self.screens[index].name) {
            self.tick_scene(index);
        }
        let screen = &self.screens[index];
        if !screen.frame_pending && (screen.fade.is_some() || screen.stale) {
            self.draw(index);
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
        let still = screen.shown.and_then(|shown| match shown.picture {
            Picture::Still(texture) => Some(texture),
            Picture::Video(_) => None,
            Picture::Shader(scene) => {
                self.drop_scene(*scene);
                None
            }
        });
        for texture in still
            .into_iter()
            .chain(screen.fade.map(|f| f.before.texture))
        {
            self.gl.delete(texture);
        }
        if let Some(viewport) = screen.viewport {
            viewport.destroy();
        }
        self.close_unused_videos();
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
            stale: false,
            failed: None,
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
