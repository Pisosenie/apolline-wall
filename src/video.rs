use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::rc::Rc;

use apolline_wall_protocol::install::{self, Need};
use glutin::api::egl::display::Display;
use glutin::display::GlDisplay;
use libloading::Library;
use smithay_client_toolkit::reexports::calloop::ping::Ping;

const PARAM_INVALID: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;
const PARAM_X11_DISPLAY: c_int = 8;
const PARAM_WL_DISPLAY: c_int = 9;

/// The display libmpv's hardware decoding works with.
#[derive(Clone, Copy)]
pub enum Platform {
    Wayland(*mut c_void),
    /// An Xlib `Display`.
    X11(*mut c_void),
}
const UPDATE_FRAME: u64 = 1;
const EVENT_NONE: c_int = 0;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_VIDEO_RECONFIG: c_int = 17;
const END_FILE_ERROR: c_int = 4;
const FORMAT_INT64: c_int = 4;
const GL_RGBA8: c_int = 0x8058;

enum Handle {}
enum RenderContext {}

#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct GlInit {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    context: *mut c_void,
}

#[repr(C)]
struct Fbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

type Callback = Option<unsafe extern "C" fn(*mut c_void)>;

/// libmpv, opened on the first video so that pictures work without it installed.
pub struct Mpv {
    create: unsafe extern "C" fn() -> *mut Handle,
    initialize: unsafe extern "C" fn(*mut Handle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut Handle),
    set_option_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    get_property: unsafe extern "C" fn(*mut Handle, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(*mut Handle, *mut *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(*mut Handle, f64) -> *mut Event,
    set_wakeup_callback: unsafe extern "C" fn(*mut Handle, Callback, *mut c_void),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    render_create: unsafe extern "C" fn(*mut *mut RenderContext, *mut Handle, *mut Param) -> c_int,
    render_set_update_callback: unsafe extern "C" fn(*mut RenderContext, Callback, *mut c_void),
    render_update: unsafe extern "C" fn(*mut RenderContext) -> u64,
    render_render: unsafe extern "C" fn(*mut RenderContext, *mut Param) -> c_int,
    render_free: unsafe extern "C" fn(*mut RenderContext),
    _library: Library,
}

impl Mpv {
    pub fn open() -> Result<Rc<Self>, String> {
        // mpv 0.35 on is libmpv.so.2; Ubuntu 22.04 and Debian 11 still ship libmpv.so.1, whose
        // render API is the same.
        let library = ["libmpv.so.2", "libmpv.so.1", "libmpv.so"]
            .iter()
            .find_map(|name| unsafe { Library::new(name) }.ok())
            .ok_or_else(|| format!("videos need libmpv: {}", install::hint(Need::Mpv)))?;
        unsafe {
            Ok(Rc::new(Self {
                create: symbol(&library, "mpv_create")?,
                initialize: symbol(&library, "mpv_initialize")?,
                terminate_destroy: symbol(&library, "mpv_terminate_destroy")?,
                set_option_string: symbol(&library, "mpv_set_option_string")?,
                set_property_string: symbol(&library, "mpv_set_property_string")?,
                get_property: symbol(&library, "mpv_get_property")?,
                command: symbol(&library, "mpv_command")?,
                wait_event: symbol(&library, "mpv_wait_event")?,
                set_wakeup_callback: symbol(&library, "mpv_set_wakeup_callback")?,
                error_string: symbol(&library, "mpv_error_string")?,
                render_create: symbol(&library, "mpv_render_context_create")?,
                render_set_update_callback: symbol(
                    &library,
                    "mpv_render_context_set_update_callback",
                )?,
                render_update: symbol(&library, "mpv_render_context_update")?,
                render_render: symbol(&library, "mpv_render_context_render")?,
                render_free: symbol(&library, "mpv_render_context_free")?,
                _library: library,
            }))
        }
    }

    fn error(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &str) -> Result<T, String> {
    let name = CString::new(name).map_err(|err| err.to_string())?;
    unsafe { library.get::<T>(name.as_bytes_with_nul()) }
        .map(|symbol| *symbol)
        .map_err(|err| format!("libmpv: {err}"))
}

pub enum Happened {
    Size(u32, u32),
    Saved(Result<(), String>),
    Failed(String),
}

/// One looping, muted video rendered by libmpv into a framebuffer of the shared GL context.
/// It must be dropped while that context is current.
pub struct Player {
    mpv: Rc<Mpv>,
    handle: *mut Handle,
    render: *mut RenderContext,
    gl_display: Box<Display>,
    wake: Box<Ping>,
}

impl Player {
    pub fn new(
        mpv: &Rc<Mpv>,
        source: &Path,
        display: &Display,
        platform: Platform,
        wake: &Ping,
    ) -> Result<Self, String> {
        let handle = unsafe { (mpv.create)() };
        if handle.is_null() {
            return Err("mpv_create failed".into());
        }
        let mut player = Self {
            mpv: mpv.clone(),
            handle,
            render: ptr::null_mut(),
            gl_display: Box::new(display.clone()),
            wake: Box::new(wake.clone()),
        };
        for (name, value) in [
            ("vo", "libmpv"),
            ("gpu-dumb-mode", "yes"),
            ("hwdec", "auto-safe"),
            ("loop-file", "inf"),
            ("aid", "no"),
            ("sid", "no"),
            ("osd-level", "0"),
            ("video-timing-offset", "0"),
            ("load-scripts", "no"),
            ("ytdl", "no"),
            ("terminal", "no"),
            // A Workshop "video" is anyone's file: one that is really a playlist, an EDL or an
            // MP4 pointing elsewhere would have mpv fetch URLs or open other files. Only
            // ffmpeg's demuxers, reading only local files, and no references out.
            ("demuxer", "lavf"),
            ("demuxer-lavf-o", "protocol_whitelist=file"),
            ("access-references", "no"),
            ("load-unsafe-playlists", "no"),
        ] {
            player.check(unsafe {
                (mpv.set_option_string)(handle, c(name).as_ptr(), c(value).as_ptr())
            })?;
        }
        player.check(unsafe { (mpv.initialize)(handle) })?;
        player.render = player.render_context(platform)?;
        let wake = &*player.wake as *const Ping as *mut c_void;
        unsafe {
            (mpv.render_set_update_callback)(player.render, Some(ping), wake);
            (mpv.set_wakeup_callback)(handle, Some(ping), wake);
        }
        let path = CString::new(source.as_os_str().as_bytes()).map_err(|err| err.to_string())?;
        let loadfile = c("loadfile");
        let mut args = [loadfile.as_ptr(), path.as_ptr(), ptr::null()];
        player.check(unsafe { (mpv.command)(handle, args.as_mut_ptr()) })?;
        Ok(player)
    }

    fn render_context(&self, platform: Platform) -> Result<*mut RenderContext, String> {
        let api = c("opengl");
        let mut init = GlInit {
            get_proc_address: proc_address,
            context: &*self.gl_display as *const Display as *mut c_void,
        };
        let mut params = [
            param(PARAM_API_TYPE, api.as_ptr().cast_mut().cast()),
            param(PARAM_OPENGL_INIT_PARAMS, (&raw mut init).cast()),
            match platform {
                Platform::Wayland(display) => param(PARAM_WL_DISPLAY, display),
                Platform::X11(display) => param(PARAM_X11_DISPLAY, display),
            },
            param(PARAM_INVALID, ptr::null_mut()),
        ];
        let mut render = ptr::null_mut();
        self.check(unsafe {
            (self.mpv.render_create)(&mut render, self.handle, params.as_mut_ptr())
        })?;
        Ok(render)
    }

    pub fn set_paused(&self, paused: bool) {
        let value = c(if paused { "yes" } else { "no" });
        unsafe { (self.mpv.set_property_string)(self.handle, c("pause").as_ptr(), value.as_ptr()) };
    }

    pub fn set_speed(&self, speed: f32) {
        let value = c(&format!("{speed}"));
        unsafe { (self.mpv.set_property_string)(self.handle, c("speed").as_ptr(), value.as_ptr()) };
    }

    pub fn has_frame(&self) -> bool {
        unsafe { (self.mpv.render_update)(self.render) & UPDATE_FRAME != 0 }
    }

    pub fn render(&self, framebuffer: u32, (w, h): (u32, u32)) -> Result<(), String> {
        let mut fbo = Fbo {
            fbo: framebuffer as c_int,
            w: w as c_int,
            h: h as c_int,
            internal_format: GL_RGBA8,
        };
        let mut flip: c_int = 0;
        let mut params = [
            param(PARAM_OPENGL_FBO, (&raw mut fbo).cast()),
            param(PARAM_FLIP_Y, (&raw mut flip).cast()),
            param(PARAM_INVALID, ptr::null_mut()),
        ];
        self.check(unsafe { (self.mpv.render_render)(self.render, params.as_mut_ptr()) })
    }

    pub fn happenings(&self) -> Vec<Happened> {
        let mut happened = Vec::new();
        loop {
            let event = unsafe { &*(self.mpv.wait_event)(self.handle, 0.0) };
            match event.id {
                EVENT_NONE => return happened,
                EVENT_VIDEO_RECONFIG => {
                    if let Some(size) = self.size() {
                        happened.push(Happened::Size(size.0, size.1));
                    }
                }
                EVENT_END_FILE => {
                    let end = unsafe { &*(event.data as *const EndFile) };
                    if end.reason == END_FILE_ERROR {
                        happened.push(Happened::Failed(self.mpv.error(end.error)));
                    }
                }
                EVENT_SHUTDOWN => happened.push(Happened::Failed("mpv shut down".into())),
                _ => {}
            }
        }
    }

    fn size(&self) -> Option<(u32, u32)> {
        let property = |name: &str| {
            let mut value: i64 = 0;
            let code = unsafe {
                (self.mpv.get_property)(
                    self.handle,
                    c(name).as_ptr(),
                    FORMAT_INT64,
                    (&raw mut value).cast(),
                )
            };
            (code >= 0 && value > 0).then_some(value as u32)
        };
        Some((property("dwidth")?, property("dheight")?))
    }

    fn check(&self, code: c_int) -> Result<(), String> {
        if code < 0 {
            Err(format!("mpv: {}", self.mpv.error(code)))
        } else {
            Ok(())
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            if !self.render.is_null() {
                (self.mpv.render_free)(self.render);
            }
            (self.mpv.terminate_destroy)(self.handle);
        }
    }
}

unsafe extern "C" fn ping(wake: *mut c_void) {
    unsafe { (*(wake as *const Ping)).ping() }
}

unsafe extern "C" fn proc_address(display: *mut c_void, name: *const c_char) -> *mut c_void {
    let display = unsafe { &*(display as *const Display) };
    display
        .get_proc_address(unsafe { CStr::from_ptr(name) })
        .cast_mut()
}

fn param(kind: c_int, data: *mut c_void) -> Param {
    Param { kind, data }
}

fn c(text: &str) -> CString {
    CString::new(text).expect("no NUL in mpv names")
}
