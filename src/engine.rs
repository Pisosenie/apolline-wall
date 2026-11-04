mod playback;
mod scenery;
mod shading;
mod socket;
mod wayland;
mod x11;

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use raw_window_handle::RawDisplayHandle;
use smithay_client_toolkit::reexports::calloop::channel::{self, Sender};
use smithay_client_toolkit::reexports::calloop::ping::{self, Ping};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};

use apolline_wall_protocol::source::{self, Kind, poster_path};

use crate::gl::{self, Canvas, Gl, Target, Texture};
use crate::picture::{self, Pixels, UPSIDE_DOWN, WHOLE};
use crate::protocol::{self, Adjust, Command, Event, Fit, Monitor, Show};
use crate::state::{self, State};
use crate::video::{Mpv, Platform, Player};

/// Apolline starts it as a child and talks over stdin; on its own it runs as a daemon and remembers.
pub enum Mode {
    Child,
    Daemon,
}

/// What the monitors' surfaces come from. Everything else, from decoding to crossfades, is
/// the same on both.
// There is one of these per process, so its size doesn't matter.
#[allow(clippy::large_enum_variant)]
enum Desktop {
    Wayland(wayland::Wayland),
    X11(x11::X11),
}

enum Surface {
    Wayland(wayland::Output),
    X11(x11::Window),
}

pub fn run(mode: Mode) -> Result<(), String> {
    share_arenas();
    shading::local_time_zone();
    let mut event_loop = EventLoop::<App>::try_new().map_err(|err| err.to_string())?;
    let handle = event_loop.handle();
    let (desktop, display, platform) = connect(&handle)?;
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

    let (endings, ends) = channel::channel();
    handle
        .insert_source(ends, |event, _, app: &mut App| {
            if let channel::Event::Msg(ended) = event {
                app.ended(ended);
            }
        })
        .map_err(|err| err.to_string())?;

    let (wake, woken) = ping::make_ping().map_err(|err| err.to_string())?;
    handle
        .insert_source(woken, |_, _, app: &mut App| app.tick())
        .map_err(|err| err.to_string())?;

    let mut app = App {
        screens: Vec::new(),
        videos: HashMap::new(),
        gl: Gl::new(display)?,
        desktop,
        platform,
        everywhere: None,
        per_output: HashMap::new(),
        paused_everywhere: false,
        paused: HashSet::new(),
        decoder,
        mpv: None,
        wake,
        endings,
        handle,
        fps: shading::DEFAULT_FPS,
        scale: None,
        accent: shading::DEFAULT_ACCENT,
        persist: None,
        generation: 0,
        next_screen: 0,
        exit: false,
    };
    if let Mode::Daemon = mode {
        app.persist = restore(&mut app);
    }
    app.discover();
    while !app.exit {
        event_loop
            .dispatch(None, &mut app)
            .map_err(|err| err.to_string())?;
        app.flush();
    }
    app.close_videos();
    app.close_externals();
    let _ = std::fs::remove_file(protocol::socket_path());
    Ok(())
}

/// Wayland when there is one, X11 otherwise. A Wayland session that can't host a background,
/// GNOME without the Shell extension, still has Xwayland, whose desktop-type windows its
/// window manager keeps under everything.
fn connect(
    handle: &LoopHandle<'static, App>,
) -> Result<(Desktop, RawDisplayHandle, Platform), String> {
    let x11 = std::env::var_os("DISPLAY").is_some();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match wayland::connect(handle) {
            Ok((wayland, display, platform)) => {
                return Ok((Desktop::Wayland(wayland), display, platform));
            }
            Err(why) if x11 => eprintln!("apolline-wall: {why}; drawing through Xwayland instead"),
            Err(why) => return Err(why),
        }
    }
    if x11 {
        let (x11, display, platform) = x11::connect(handle)?;
        return Ok((Desktop::X11(x11), display, platform));
    }
    Err("no display to draw on: neither WAYLAND_DISPLAY nor DISPLAY is set".into())
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
    desktop: Desktop,
    /// What libmpv is told about the display, for hardware decoding.
    platform: Platform,
    everywhere: Option<Show>,
    per_output: HashMap<String, Show>,
    paused_everywhere: bool,
    paused: HashSet<String>,
    decoder: mpsc::Sender<Batch>,
    mpv: Option<Rc<Mpv>>,
    wake: Ping,
    endings: Sender<scenery::Ended>,
    handle: LoopHandle<'static, App>,
    fps: u32,
    scale: Option<f32>,
    accent: [f32; 3],
    persist: Option<Persist>,
    generation: u64,
    next_screen: u64,
    exit: bool,
}

/// The daemon's memory: what to restore, and what last showed for when a wallpaper fails.
struct Persist {
    path: PathBuf,
    state: State,
    good: State,
}

struct Screen {
    /// Stays the same while the monitor is there, for work that comes back later.
    id: u64,
    target: Option<Target>,
    name: String,
    surface: Surface,
    shown: Option<Shown>,
    fade: Option<Fade>,
    loading: Option<Loading>,
    frame_pending: bool,
    stale: bool,
    failed: Option<Show>,
    /// Our surface is unmapped while another program draws this monitor.
    hidden: bool,
}

enum Picture {
    Still(Texture),
    Video(PathBuf),
    Shader(Box<Scene>),
    External(scenery::External),
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
    screen: u64,
    generation: u64,
    size: (u32, u32),
}

struct Loaded {
    screen: u64,
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
        // Every client hears what changed, not only the one that asked.
        let announce = matches!(
            command,
            Command::Pause(_) | Command::Limits(_) | Command::Uniforms(_) | Command::Adjust(_)
        );
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
                self.sync_externals();
                self.pause_scenes();
                for index in 0..self.screens.len() {
                    if self.screens[index].fade.is_some() {
                        self.draw(index);
                    }
                }
            }
            Command::Limits(limits) => self.limit(limits),
            Command::Uniforms(uniforms) => self.tint(uniforms),
            Command::Adjust(adjust) => self.adjust(adjust),
            Command::Status {} => protocol::emit(&self.status()),
        }
        if announce {
            protocol::emit(&self.status());
        }
    }

    /// A new look for what is shown now, kept in the wanted wallpapers so a later refresh
    /// doesn't take it back.
    fn adjust(&mut self, adjust: Adjust) {
        let look = Some(adjust.look);
        match &adjust.output {
            None => {
                for show in self
                    .everywhere
                    .iter_mut()
                    .chain(self.per_output.values_mut())
                {
                    show.look = look;
                }
            }
            Some(name) => match (self.per_output.get_mut(name), &self.everywhere) {
                (Some(show), _) => show.look = look,
                (None, Some(everywhere)) => {
                    let show = Show {
                        output: Some(name.clone()),
                        look,
                        ..everywhere.clone()
                    };
                    self.per_output.insert(name.clone(), show);
                }
                (None, None) => {}
            },
        }
        for index in 0..self.screens.len() {
            let screen = &mut self.screens[index];
            if adjust
                .output
                .as_ref()
                .is_none_or(|name| *name == screen.name)
                && let Some(shown) = &mut screen.shown
            {
                shown.show.look = look;
                self.restyle(index);
            }
        }
    }

    /// Brings a video's pace in line with its look and redraws.
    fn restyle(&mut self, index: usize) {
        if let Some(shown) = &self.screens[index].shown
            && let Picture::Video(source) = &shown.picture
            && let Some(video) = self.videos.get(source)
        {
            video
                .player
                .set_speed(shown.show.look.unwrap_or_default().speed);
        }
        self.draw(index);
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
                    look: screen
                        .shown
                        .as_ref()
                        .and_then(|shown| shown.show.look)
                        .filter(|look| !look.is_plain()),
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
        let mut externals = Vec::new();
        let mut restyled = Vec::new();
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
                    && (matches!(kind, Kind::Video | Kind::Scene) || latest_size == size)
            }) {
                if let Some(shown) = &mut self.screens[index].shown
                    && shown.show.look.unwrap_or_default() != show.look.unwrap_or_default()
                {
                    shown.show.look = show.look;
                    restyled.push(index);
                }
                continue;
            }
            self.generation += 1;
            match kind {
                Kind::Video => videos.push(index),
                Kind::Shader => scenes.push(index),
                Kind::Scene => externals.push(index),
                Kind::Picture => jobs
                    .entry((show.source.clone(), show.fit))
                    .or_default()
                    .push(Job {
                        screen: screen.id,
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
        for index in externals {
            self.start_external(index);
        }
        for index in restyled {
            self.restyle(index);
        }
        self.fit_canvases();
    }

    fn loaded(&mut self, loaded: Loaded) {
        let Some(index) = self.index_of(loaded.screen) else {
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
        let was_external = matches!(
            self.screens[index]
                .shown
                .as_ref()
                .map(|shown| &shown.picture),
            Some(Picture::External(_))
        );
        let external = matches!(picture, Picture::External(_));
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
            Some(before) if !paused && !external && !duration.is_zero() => {
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
        if was_external && !external {
            self.unhide(index);
        }
        self.draw(index);
        self.close_unused_videos();
        self.sync_pauses();
    }

    fn retire(&self, index: usize, old: Shown) -> Option<gl::Layer> {
        match old.picture {
            Picture::Still(texture) => Some(gl::Layer::new(texture, WHOLE, old.show.look)),
            Picture::Video(source) => {
                let canvas = self.videos.get(&source)?.canvas.as_ref()?;
                let target = self.screens[index].target.as_ref()?;
                Some(gl::Layer::new(
                    self.gl.snapshot(canvas).ok()?,
                    picture::placement(old.show.fit, canvas.size, target.size()),
                    old.show.look,
                ))
            }
            Picture::Shader(scene) => {
                let texture = self.gl.snapshot(&scene.canvas).ok();
                self.drop_scene(*scene);
                Some(gl::Layer::new(texture?, UPSIDE_DOWN, old.show.look))
            }
            Picture::External(external) => {
                scenery::stop(&external);
                None
            }
        }
    }

    fn draw(&mut self, index: usize) {
        let paused = self.is_paused(&self.screens[index].name);
        let screen = &self.screens[index];
        let (Some(target), Some(shown)) = (&screen.target, &screen.shown) else {
            return;
        };
        let after = match &shown.picture {
            Picture::Still(texture) => gl::Layer::new(*texture, WHOLE, shown.show.look),
            Picture::Video(source) => {
                let Some(canvas) = self.videos.get(source).and_then(|v| v.canvas.as_ref()) else {
                    return;
                };
                gl::Layer::new(
                    canvas.texture,
                    picture::placement(shown.show.fit, canvas.size, target.size()),
                    shown.show.look,
                )
            }
            Picture::Shader(scene) => {
                gl::Layer::new(scene.canvas.texture, UPSIDE_DOWN, shown.show.look)
            }
            Picture::External(_) => return,
        };
        let progress = match &screen.fade {
            Some(fade) if !paused => {
                fade.start.elapsed().as_secs_f32() / fade.duration.as_secs_f32()
            }
            _ => 1.0,
        };
        let fading = progress < 1.0;
        let playing = matches!(shown.picture, Picture::Video(_) | Picture::Shader(_)) && !paused;
        let before = screen
            .fade
            .as_ref()
            .filter(|_| fading)
            .map(|fade| fade.before);
        if (fading || playing) && !screen.frame_pending {
            self.want_frame(index);
        }
        let screen = &mut self.screens[index];
        screen.stale = false;
        let Some(target) = &screen.target else {
            return;
        };
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
        match &self.screens[index].surface {
            Surface::Wayland(_) => self.wayland_buffer_size(index),
            Surface::X11(window) => Some(window.size()),
        }
    }

    /// Asks to hear when the monitor is ready for another frame.
    fn want_frame(&mut self, index: usize) {
        match &self.screens[index].surface {
            Surface::Wayland(_) => self.wayland_frame(index),
            Surface::X11(_) => self.x11_frame(index),
        }
        self.screens[index].frame_pending = true;
    }

    fn place(&mut self, index: usize) {
        let Some(size) = self.buffer_size(index) else {
            return;
        };
        let window = match &self.screens[index].surface {
            Surface::Wayland(_) => self.wayland_prepare(index),
            Surface::X11(window) => Some(window.handle()),
        };
        let Some(window) = window else {
            return;
        };
        let screen = &mut self.screens[index];
        let placed = match &mut screen.target {
            Some(target) if target.size() == size => Ok(()),
            Some(target) => self.gl.resize(target, size),
            None => self
                .gl
                .target(window, size)
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

    fn add_screen(&mut self, name: String, surface: Surface) -> usize {
        self.next_screen += 1;
        self.screens.push(Screen {
            id: self.next_screen,
            target: None,
            name,
            surface,
            shown: None,
            fade: None,
            loading: None,
            frame_pending: false,
            stale: false,
            failed: None,
            hidden: false,
        });
        self.screens.len() - 1
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.screens.iter().position(|screen| screen.id == id)
    }

    /// X11 lists its monitors up front; Wayland announces them as events.
    fn discover(&mut self) {
        if let Desktop::X11(_) = self.desktop {
            self.x11_monitors();
        }
    }

    /// X11 also answers requests made while drawing, and those can carry events with them
    /// that no readiness of its socket would announce.
    fn flush(&mut self) {
        if let Desktop::X11(_) = self.desktop {
            self.x11_events();
        }
        if let Desktop::X11(x11) = &self.desktop {
            x11.flush();
        }
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
            Picture::External(external) => {
                scenery::stop(&external);
                None
            }
        });
        for texture in still
            .into_iter()
            .chain(screen.fade.map(|f| f.before.texture))
        {
            self.gl.delete(texture);
        }
        match screen.surface {
            Surface::Wayland(output) => self.wayland_close(output),
            Surface::X11(window) => self.x11_close(window),
        }
        self.close_unused_videos();
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
                screen: job.screen,
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
