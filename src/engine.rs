mod audio;
mod crashes;
mod diagnostics;
mod feed;
mod monitors;
mod native;
mod playback;
mod pointer;
mod policy;
mod poster;
mod presentation;
mod process;
mod rotation;
mod scenery;
mod selection;
mod shading;
mod shared;
mod socket;
mod startup;
mod system;
mod wayland;
mod x11;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::calloop::channel::{self, Sender};
use smithay_client_toolkit::reexports::calloop::ping::{self, Ping};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};

use apolline_wall_protocol::source::{self, Kind};

use crate::gl::{self, Canvas, Gl, Target, Texture};
use crate::loading::{self, Batch, Job, Pending};
use crate::protocol::{self, Adjust, Command, Event, Monitor, Show};
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
    let (desktop, display, platform) = startup::connect(&handle)?;
    socket::serve(&handle)?;

    if let Mode::Child = mode {
        let (commands, incoming) = channel::channel();
        std::thread::spawn(move || startup::read_commands(commands));
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

    // Backpressure keeps decoded scenes and full-sized pixel buffers out of an unbounded queue.
    let (loads, loaded) = mpsc::sync_channel(2);
    let (load_wake, load_woken) = ping::make_ping().map_err(|err| err.to_string())?;
    handle
        .insert_source(load_woken, move |_, _, app: &mut App| {
            while let Ok(loaded) = loaded.try_recv() {
                app.loaded(loaded);
            }
        })
        .map_err(|err| err.to_string())?;
    let (decoder, batches) = mpsc::channel();
    std::thread::spawn(move || {
        loading::run(batches, |loaded| {
            if loads.send(loaded).is_err() {
                return false;
            }
            load_wake.ping();
            true
        })
    });

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

    // Events go to stdout in both modes: the parent reads them in child mode, the journal
    // keeps them for a daemon. Only a child ends when nobody reads them any more.
    let child = matches!(mode, Mode::Child);
    let output_wake = wake.clone();
    apolline_wall_protocol::output::start(move || output_wake.ping())?;
    let (posters, jobs) = mpsc::sync_channel(2);
    let (posted, results) = channel::channel();
    handle
        .insert_source(results, |event, _, app: &mut App| {
            if let channel::Event::Msg(saved) = event {
                app.posted(saved);
                app.pump_posters();
            }
        })
        .map_err(|err| err.to_string())?;
    std::thread::spawn(move || poster::run(jobs, posted));
    let (system_updates, system_events) = channel::channel();
    handle
        .insert_source(system_events, |event, _, app: &mut App| {
            if let channel::Event::Msg(update) = event {
                app.system(update);
            }
        })
        .map_err(|err| err.to_string())?;
    system::start(system_updates);
    let (feed_changes, feed_events) = channel::channel();
    handle
        .insert_source(feed_events, |event, _, app: &mut App| {
            if let channel::Event::Msg(change) = event {
                app.feed_changed(change);
            }
        })
        .map_err(|err| err.to_string())?;
    let mut app = App {
        screens: Vec::new(),
        videos: HashMap::new(),
        scenes: HashMap::new(),
        pointed_last: None,
        gl: Gl::new(display)?,
        desktop,
        platform,
        selection: selection::Selection::default(),
        manual: crate::power::ManualPause::default(),
        power: crate::power::Power::default(),
        decoder,
        posters,
        pending_posters: HashMap::new(),
        audio: None,
        deaf: false,
        kinds: HashMap::new(),
        pipewire: None,
        feed_changes,
        mpv: None,
        wake,
        endings,
        handle,
        fps: shading::DEFAULT_FPS,
        scale: None,
        accent: shading::DEFAULT_ACCENT,
        persist: None,
        rotations: HashMap::new(),
        trying: Default::default(),
        rotation_timer: None,
        generation: 0,
        next_screen: 0,
        exit: false,
    };
    if let Mode::Daemon = mode {
        app.persist = startup::restore(&mut app);
        app.recover();
    }
    app.discover();
    while !app.exit {
        event_loop
            .dispatch(None, &mut app)
            .map_err(|err| err.to_string())?;
        if child && apolline_wall_protocol::output::failed() {
            app.close_videos();
            app.close_externals();
            let _ = std::fs::remove_file(protocol::socket_path());
            return Err(
                "parent stopped consuming stdout events; restart the child and request status"
                    .into(),
            );
        }
        app.flush();
    }
    app.done_trying();
    app.close_videos();
    app.close_externals();
    let _ = std::fs::remove_file(protocol::socket_path());
    Ok(())
}

struct App {
    screens: Vec<Screen>,
    videos: HashMap<PathBuf, Video>,
    /// Moving native scenes, each drawn once for all the monitors showing it.
    scenes: HashMap<shared::SceneKey, shared::Shared>,
    /// The monitor the pointer was last seen over.
    pointed_last: Option<u64>,
    gl: Gl,
    desktop: Desktop,
    /// What libmpv is told about the display, for hardware decoding.
    platform: Platform,
    selection: selection::Selection,
    manual: crate::power::ManualPause,
    power: crate::power::Power,
    decoder: mpsc::Sender<Batch>,
    posters: mpsc::SyncSender<poster::Job>,
    pending_posters: HashMap<String, poster::Job>,
    /// What's playing, heard while a scene shown moves to it.
    audio: Option<audio::Listener>,
    /// Hearing failed, and isn't tried again until no scene listens.
    deaf: bool,
    /// What each project folder shown is, and when its `project.json` last changed.
    kinds: HashMap<PathBuf, (std::time::SystemTime, Kind)>,
    /// Plasma's wallpaper plugin's way in, once it has asked for a monitor.
    pipewire: Option<feed::Pipewire>,
    feed_changes: Sender<feed::Change>,
    mpv: Option<Rc<Mpv>>,
    wake: Ping,
    endings: Sender<scenery::Ended>,
    handle: LoopHandle<'static, App>,
    fps: u32,
    scale: Option<f32>,
    accent: [f32; 3],
    persist: Option<Persist>,
    rotations: HashMap<Option<String>, rotation::Rotation>,
    trying: crashes::Trying,
    rotation_timer: Option<smithay_client_toolkit::reexports::calloop::RegistrationToken>,
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
    covered: bool,
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
    /// Where the pointer was last over it, -1 to 1 across it with y up.
    pointer: [f32; 2],
    /// Its frames go to Plasma's wallpaper plugin instead of its own surface.
    feed: Option<feed::Feed>,
}

enum Picture {
    Still(Texture),
    Native(Texture),
    Video(PathBuf),
    Shader(Box<Scene>),
    /// A moving native scene, shared with every monitor showing it.
    Scene(shared::SceneKey),
    External(scenery::External),
}

impl Picture {
    fn native(&self) -> bool {
        matches!(self, Self::Native(_)) || self.native_animated()
    }

    fn native_animated(&self) -> bool {
        matches!(self, Self::Scene(_))
            || matches!(self, Self::Shader(scene) if matches!(scene.shader, SceneShader::Native(_, _)))
    }
}

struct Shown {
    generation: u64,
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
    _pending: Pending,
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
    unannounced: crate::announcement::Pending,
}

/// A shader drawn into its own canvas for one monitor, at a capped rate.
struct Scene {
    shader: SceneShader,
    canvas: Canvas,
    clock: crate::animation::Clock,
    timer: Option<smithay_client_toolkit::reexports::calloop::RegistrationToken>,
    /// When a native scene last followed the pointer, which eases its parallax in real time.
    followed: Option<std::time::Instant>,
}

enum SceneShader {
    Toy(gl::Shader),
    Native(Box<apolline_scene::Renderer>, protocol::Fit),
}

impl App {
    /// What a source is shown as. A project folder's `project.json` is read again only once
    /// it changes, not on every refresh of every monitor.
    fn kind_of(&mut self, path: &std::path::Path) -> Kind {
        if !path.is_dir() {
            return source::kind(path);
        }
        let changed = std::fs::metadata(path.join("project.json")).and_then(|meta| meta.modified());
        let Ok(changed) = changed else {
            return source::kind(path);
        };
        if let Some(&(when, kind)) = self.kinds.get(path)
            && when == changed
        {
            return kind;
        }
        let kind = source::kind(path);
        self.kinds.insert(path.to_path_buf(), (changed, kind));
        kind
    }

    fn command(&mut self, command: Command) {
        if let Command::Show(show) = &command
            && !show.properties.is_empty()
            && self.kind_of(&show.source) != Kind::Scene
        {
            protocol::emit(&Event::Error {
                output: show.output.clone(),
                source: Some(show.source.clone()),
                message: "user properties are supported for native scene wallpapers".into(),
            });
            return;
        }
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
            Command::Policy(_)
                | Command::Pause(_)
                | Command::Limits(_)
                | Command::Uniforms(_)
                | Command::Adjust(_)
        );
        match command {
            Command::Session(session) => {
                self.power.conditions.shell_locked = session.locked;
                self.sync_policy();
                protocol::emit(&self.status());
            }
            Command::Policy(policy) => {
                self.power.policy = policy;
                self.sync_policy();
            }
            Command::Playlist(list) => self.playlist(list),
            Command::Step(step) => self.step_playlist(step),
            Command::Show(show) => {
                match &show.output {
                    None => self.rotations.clear(),
                    Some(output) => {
                        self.rotations.remove(&Some(output.clone()));
                    }
                }
                for screen in &mut self.screens {
                    if show.output.as_ref().is_none_or(|name| *name == screen.name) {
                        screen.failed = None;
                    }
                }
                self.forgive(&show.source);
                self.selection.set(show);
                self.refresh();
            }
            Command::Pause(pause) => {
                self.manual.set(pause.output.as_deref(), pause.paused);
                self.sync_policy();
            }
            Command::Limits(limits) => self.limit(limits),
            Command::Uniforms(uniforms) => self.tint(uniforms),
            Command::Adjust(adjust) => self.adjust(adjust),
            Command::Feed(feed) => self.feed(&feed.output, feed.covered),
            Command::Status {} => protocol::emit(&self.status()),
        }
        if announce {
            protocol::emit(&self.status());
        }
    }

    /// A new look for what is shown now, kept in the wanted wallpapers so a later refresh
    /// doesn't take it back.
    fn adjust(&mut self, adjust: Adjust) {
        self.selection.adjust(&adjust);
        let look = Some(adjust.look).filter(|look| !look.is_plain());
        for index in 0..self.screens.len() {
            let screen = &mut self.screens[index];
            if adjust
                .output
                .as_ref()
                .is_none_or(|name| *name == screen.name)
            {
                if let Some(loading) = &mut screen.loading {
                    loading.show.look = look;
                }
                if let Some(shown) = &mut screen.shown {
                    shown.show.look = look;
                    self.restyle(index);
                }
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
            power: apolline_wall_protocol::power::Status {
                policy: self.power.policy.clone(),
                conditions: self.power.conditions,
                effective_fps: self.effective_fps(),
            },
            outputs: self
                .screens
                .iter()
                .enumerate()
                .map(|(index, screen)| Monitor {
                    name: screen.name.clone(),
                    source: screen.shown.as_ref().map(|shown| shown.show.source.clone()),
                    paused: self.is_paused(&screen.name),
                    manual_paused: self.manual_paused(&screen.name),
                    pause_reason: self.pause_reason(&screen.name).map(str::to_owned),
                    playlist: self.rotation_key(&screen.name).is_some(),
                    playlist_status: self.rotation_status(&screen.name),
                    native_scene: screen
                        .shown
                        .as_ref()
                        .is_some_and(|shown| shown.picture.native()),
                    animated_scene: screen
                        .shown
                        .as_ref()
                        .is_some_and(|shown| shown.picture.native_animated()),
                    look: screen
                        .shown
                        .as_ref()
                        .and_then(|shown| shown.show.look)
                        .filter(|look| !look.is_plain()),
                    feed: self.feed_stream(index),
                })
                .collect(),
            fps: self.fps,
            scale: self.scale,
        }
    }

    fn wanted(&self, index: usize) -> Option<Show> {
        self.selection.wanted(&self.screens[index].name)
    }

    fn refresh(&mut self) {
        let mut jobs: HashMap<PathBuf, Vec<Job>> = HashMap::new();
        let mut videos = Vec::new();
        let mut scenes = Vec::new();
        let mut restyled = Vec::new();
        for index in 0..self.screens.len() {
            let (Some(size), Some(show)) = (self.buffer_size(index), self.wanted(index)) else {
                continue;
            };
            let kind = self.kind_of(&show.source);
            let screen = &self.screens[index];
            if screen.failed.as_ref() == Some(&show) {
                continue;
            }
            if !show.properties.is_empty() && kind != Kind::Scene {
                let source = show.source.clone();
                let output = self.screens[index].name.clone();
                self.give_up(index, show);
                protocol::emit(&Event::Error {
                    output: Some(output),
                    source: Some(source),
                    message: "user properties are supported for native scene wallpapers".into(),
                });
                continue;
            }
            let latest = match (&screen.loading, &screen.shown) {
                (Some(loading), _) => Some((&loading.show, loading.size)),
                (None, Some(shown)) => Some((&shown.show, shown.size)),
                (None, None) => None,
            };
            if latest.is_some_and(|(latest, latest_size)| {
                latest.source == show.source
                    && latest.properties == show.properties
                    && latest.fit == show.fit
                    && (kind == Kind::Video
                        || matches!(
                            screen.shown.as_ref().map(|s| &s.picture),
                            Some(Picture::External(_) | Picture::Scene(_))
                        )
                        || latest_size == size)
            }) {
                if let Some(loading) = &mut self.screens[index].loading {
                    loading.show.look = show.look;
                }
                if let Some(shown) = &mut self.screens[index].shown
                    && shown.show.look.unwrap_or_default() != show.look.unwrap_or_default()
                {
                    shown.show.look = show.look;
                    restyled.push(index);
                }
                continue;
            }
            self.generation += 1;
            let pending = Pending::default();
            match kind {
                Kind::Video => videos.push(index),
                Kind::Shader => scenes.push(index),
                Kind::Scene | Kind::Picture => {
                    jobs.entry(show.source.clone()).or_default().push(Job {
                        screen: screen.id,
                        generation: self.generation,
                        size,
                        fit: show.fit,
                        properties: show.properties.clone(),
                        ticket: pending.ticket(),
                    })
                }
            }
            self.screens[index].loading = Some(Loading {
                _pending: pending,
                generation: self.generation,
                show,
                size,
            });
        }
        for (source, jobs) in jobs {
            let _ = self.decoder.send(Batch { source, jobs });
        }
        self.note_trying();
        for index in videos {
            self.start_video(index);
        }
        for index in scenes {
            self.start_scene(index);
        }
        for index in restyled {
            self.restyle(index);
        }
        self.close_unused_videos();
        self.fit_canvases();
        self.fit_scenes();
        self.schedule_rotations();
    }

    fn give_up(&mut self, index: usize, show: Show) {
        let mut back = None;
        if let Some(persist) = &mut self.persist
            && persist.state.undo(&show, &persist.good)
        {
            if let Err(message) = state::save(&persist.path, &persist.state) {
                protocol::emit(&Event::Error {
                    output: None,
                    source: None,
                    message,
                });
            }
            // The last wallpaper that showed comes back on screen too, not only into the file.
            back = persist
                .state
                .commands()
                .into_iter()
                .find_map(|command| match command {
                    Command::Show(saved) if saved.output == show.output => Some(saved),
                    _ => None,
                });
        }
        self.rotation_failed(index, &show);
        self.screens[index].failed = Some(show);
        if let Some(back) = back {
            match &back.output {
                None => self.selection.everywhere = Some(back),
                Some(output) => {
                    self.selection.per_output.insert(output.clone(), back);
                }
            }
            // Not from inside whatever load or draw is failing right now.
            let _ = self.handle.insert_idle(|app| app.refresh());
        }
        // A wallpaper that failed on its own didn't take the daemon down.
        self.note_trying();
        protocol::emit(&self.status());
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
            _ if self.screens[index].feed.is_some() => self.feed_frame(index),
            Surface::Wayland(_) => self.wayland_frame(index),
            Surface::X11(_) => self.x11_frame(index),
        }
        self.screens[index].frame_pending = true;
    }

    fn place(&mut self, index: usize) {
        let Some(size) = self.buffer_size(index) else {
            return;
        };
        if self.screens[index].feed.is_some() {
            return self.place_feed(index, size);
        }
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

    /// `identity` is what the compositor calls the monitor, when it says, so its settings
    /// follow it from one connector to another.
    fn add_screen(&mut self, name: String, surface: Surface, identity: Option<String>) -> usize {
        if let Some(identity) = identity {
            self.recognise(&name, &identity);
        }
        self.next_screen += 1;
        self.screens.push(Screen {
            covered: false,
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
            pointer: [0.0; 2],
            feed: None,
        });
        self.screens.len() - 1
    }

    fn release_target(&mut self, index: usize) {
        if let Some(target) = self.screens[index].target.take() {
            self.gl.release(target);
        }
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
        // Replies X11 sends while events are being handled can carry more events, which
        // no readiness of its socket would announce, so it drains until a pass finds none.
        if let Desktop::X11(_) = self.desktop {
            for _ in 0..8 {
                if self.x11_events() == 0 {
                    break;
                }
            }
        }
        if let Desktop::X11(x11) = &self.desktop {
            x11.flush();
        }
    }

    fn forget(&mut self, index: usize) {
        let mut screen = self.screens.remove(index);
        self.pending_posters.remove(&screen.name);
        if let Some(target) = screen.target.take() {
            self.gl.release(target);
        }
        let still = screen.shown.and_then(|shown| match shown.picture {
            Picture::Still(texture) | Picture::Native(texture) => Some(texture),
            Picture::Video(_) | Picture::Scene(_) => None,
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
        self.close_unused_scenes();
        self.fit_scenes();
        self.schedule_rotations();
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
