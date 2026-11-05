# apolline-wall

Live wallpapers for Linux: pictures, videos, Shadertoy shaders and Wallpaper Engine's wallpapers, its scenes included, on every monitor, crossfading between them and costing nothing while nothing shows. It draws on its own layer-shell surface per monitor with GLES, at each monitor's physical resolution, so it runs on niri, Hyprland, sway, river, labwc, Wayfire and COSMIC; on GNOME, which has no such layer, its Shell extension in `gnome/` puts a window per monitor behind everything instead, and in X11 sessions it draws in a desktop window per monitor. It is also the wallpaper process of [Apolline](https://github.com/Pisosenie/apolline), which starts it and drives it, so video decoding never ends up inside the shell. Arch first; see [`docs/ROADMAP.md`](docs/ROADMAP.md) for what comes next.

## Using it

Install `apolline-wall-git` from the AUR (or `packaging/arch` here); on Debian 12 or Ubuntu 22.04 and later, the `.deb` from a release (`sudo apt install ./apolline-wall_*.deb`). Stop whatever draws your wallpaper now (hyprpaper, swww, swaybg), and start the daemon with your session:

| Session | Line |
|---|---|
| niri | `spawn-at-startup "apolline-wall" "daemon"` in `config.kdl` |
| Hyprland | `exec-once = apolline-wall daemon` |
| sway | `exec apolline-wall daemon` |
| any session with systemd's `graphical-session.target` | `systemctl --user enable --now apolline-wall` |
| GNOME | `gnome-extensions enable apolline-wall@pisosenie.github.io`, log out and back in, then the systemd line above |

Then pick wallpapers in the **Wallpapers** app (`apolline-wallpapers`), or from a terminal:

```sh
apolline-wall set ~/Pictures/dunes.jpg                  # every monitor
apolline-wall set ~/Videos/rain.mp4 --output DP-1       # one monitor
apolline-wall set ~/shaders/hills.frag --transition 0
apolline-wall set ~/.local/share/Steam/steamapps/workshop/content/431960/1234567890   # a scene
apolline-wall limits --fps 20 --scale 0.5               # how hard shaders may work
apolline-wall pause                                      # and resume
apolline-wall status
```

`set` waits until the wallpaper shows and says why if it can't. Each monitor's wallpaper follows the monitor: plug it into another port, or into a dock, and it comes back with the same wallpaper, look and playlist (two identical monitors without serials can't be told apart). The daemon remembers what each monitor shows and the limits in `~/.config/apolline-wall/state.toml`, so a login comes back to the same desktop. A wallpaper that fails is not written there; the last one that showed stays. One that takes the daemon down with it twice in a row is skipped at the next start, with an error saying so, until you set it again. Under Apolline the shell decides instead.

## The app

`apolline-wallpapers` is a window to browse and set wallpapers, in Apolline's look, on any desktop. It talks to the daemon over the same socket as the CLI, and starts the daemon if it isn't running.

- **Library.** Pictures, videos and shaders from `~/Pictures/Wallpapers`, `~/Videos/Wallpapers`, `~/.local/share/apolline-wall/wallpapers` and any folder you add in Settings.
- **Wallpaper Engine.** Everything Wallpaper Engine has on this machine: Workshop subscriptions and its own projects, from every Steam library, the native client and the Flatpak alike. Install Wallpaper Engine through Steam with Proton and subscribe in the Workshop as you would on Windows. Its videos and supported static 2D scenes play here; other scenes and web wallpapers use linux-wallpaperengine (see [Scenes](#scenes)). Unsupported entries say what to install. Application wallpapers are Windows programs and say so.
- **Steam Workshop.** Wallpaper Engine's Workshop, browsed without an account: videos, scenes, everything or collections, trending, newest or most subscribed, and search. A collection opens to what it holds, and **More from the author** lists the rest of an author's work. Items rated Questionable show only if you turn them on in Settings; Mature ones never do. Collections have no rating of their own, so one shows only if the first wallpapers in it pass. `apolline-wallpapers --page workshop` opens straight on this tab. Two ways to get an item:
  - With the Steam client, **Subscribe in Steam** opens the item there; once Steam has downloaded it, it appears in your library by itself and can be set from the same place.
  - Without it, **Download** goes through `steamcmd` (the `steamcmd` package in the AUR) with a Steam account that owns Wallpaper Engine. Sign in once under **Steam account** in Settings: steamcmd asks for the password and the Steam Guard code, the app shows its question and hands your answer straight to it, and steamcmd keeps the sign-in. The app saves only the account name. Downloads land in `~/.local/share/apolline-wall/steamcmd`, one at a time, and each time the Workshop tab opens the app asks the Workshop which of them have a newer version, for an **Update** button.
- **Inspector.** Pick a wallpaper, then every monitor or one, and whether it fills the screen, shows whole, or shows whole with its edges filled by a blurred copy of itself.
- **Look.** Brightness, contrast, saturation and a mirror for pictures, videos, shaders and active native still scenes, and speed for videos and shaders. While the wallpaper is on the chosen monitor every change shows there at once as a draft; **Apply** keeps it, **Revert** goes back. Each wallpaper remembers its look.
- **Tray.** `apolline-wallpapers --tray` puts it in the tray without the window: pause, resume and a random wallpaper for each monitor, and the window a click away. Settings can start it at login through XDG autostart; on niri or Hyprland add the command to your startup.
- **Now playing.** What each monitor shows, with pause.
- **Report a problem.** "Not working? Report it" saves a plain-text report about the wallpaper, the system and the daemon's last errors to Downloads, with your home folder and user name taken out, ready to attach to an issue.

Thumbnails are cached in `~/.cache/apolline-wall`; videos get theirs from ffmpeg, shaders once the daemon has shown them, and animated Workshop previews a frame from their middle. Workshop previews are kept to 150 MB, oldest out first. Workshop titles in Chinese, Japanese or Korean need a CJK font such as `noto-fonts-cjk`.

## Talking to it

Every instance listens on `$XDG_RUNTIME_DIR/apolline-wall/control.sock`, one JSON command per line in and one event per line out; a second instance finds the socket alive and leaves. Apolline starts it without arguments and talks over stdin and stdout instead, and then it exits when stdin closes, so it never outlives the shell.

| Command | Does |
|---|---|
| `{"show":{"source":"/abs/path.jpg"}}` | every monitor; a folder is a Wallpaper Engine project; also `"output":"DP-1"` for one, `"fit":"cover"\|"contain"\|"blur"`, `"transition_ms":400` |
| `{"pause":{"paused":true}}` | no animation, videos and shaders stop, changes land at once; `"output"` for one monitor |
| `{"limits":{"fps":30,"scale":0.5}}` | how hard shaders may work: frames per second (1–240, 30 to start with) and the share of the monitor's resolution they render at (0.1–1); either can be left out |
| `{"uniforms":{"accent":"#86bdf0"}}` | the shell's accent, which shaders read as `iAccent` |
| `{"adjust":{"look":{"brightness":-0.2,"contrast":1,"saturation":1,"flip":false,"speed":1},"draft":true}}` | the look of what is shown now; fields left out keep their defaults; `"output"` for one monitor; a draft shows but isn't remembered. `show` takes the same `look` |
| `{"status":{}}` | answered to whoever asked with `{"status":{"outputs":[{"name","source","paused"}],"fps","scale"}}` |

Events go to stdout and to every socket client, and after `pause`, `limits`, `uniforms` and `adjust` every client also gets a fresh `status`, so a tray, the app and the shell never disagree. It answers `{"ready":{"output","source","poster"}}` once a monitor shows the new wallpaper, where `poster` is a still image of it, and `{"error":{"output","source","message"}}` when a file can't be read or a line isn't a command. A file that failed is not retried until the next `show`. Static pictures cost nothing between changes: no frames are drawn and no timers run.

## Videos

A `source` ending in `.mp4`, `.webm`, `.mkv`, `.mov`, `.gif` and the like plays muted and looping through libmpv, with hardware decoding where the driver offers it. libmpv is opened only when the first video is shown, so pictures work without mpv installed and a video then answers with an error saying so.

- One decode feeds every monitor showing the same file, at the size the most demanding monitor needs and never above the video's own.
- A frame is rendered only when a monitor showing the video is ready for one. When a fullscreen window covers the wallpaper the compositor stops asking, and decoding stops with it: in the test VM a 1080p clip went from about seven cores to under one, under both sway and niri.
- mpv runs in its simplest rendering mode: no HDR tone mapping or fancy scalers, which a wallpaper doesn't need and a laptop battery would pay for.
- The poster frame is written to `$XDG_RUNTIME_DIR/apolline-wall/poster-*.png` before `ready` is sent, for the shell's accent and the lock screen.

## Shaders

A `source` ending in `.frag` or `.glsl` is a fragment shader written for [Shadertoy](https://www.shadertoy.com): a `mainImage(out vec4 fragColor, in vec2 fragCoord)` with `iTime`, `iTimeDelta`, `iFrame`, `iFrameRate`, `iResolution`, `iDate` and `iChannel0`–`iChannel3`, which all hold a tiling noise texture since there is nowhere to pick another yet. On top of those, `iAccent` is the shell's accent as RGB in 0–1, so a wallpaper can follow the mode you're in. Most single-pass shaders from Shadertoy paste in as they are; multi-pass buffers, sound and keyboard input are not supported. Mind their licenses: Shadertoy's default is CC BY-NC-SA. [`tests/shaders/hills.frag`](tests/shaders/hills.frag) is one of ours.

- A shader draws into a canvas of its own per monitor, about a 1080p screen's worth of pixels unless `limits` says otherwise, and the result is stretched over the monitor. It draws at most `fps` times a second and only when the compositor asks for a frame, so a covered or paused shader costs nothing and no timers keep running.
- A paused shader keeps its clock still and resumes where it stopped; a new accent shows at once, on paused shaders too.
- A shader that doesn't compile comes back as an `error` with the compiler's message, and its line numbers point into your file.
- The poster frame is its first frame, written as a PNG like a video's.

## Scenes

Supported 2D scenes are drawn directly by `apolline-scene` in the daemon's GLES context.
CPU preparation runs off the event loop. Static materials are flattened once at monitor
resolution; active time-driven fragments reuse the shader scheduler, FPS/quality limits and
pause rules. Crossfades, cover/contain, brightness, contrast, saturation and mirroring apply to
both paths. Status marks them with `native_scene: true`; animated materials additionally set
`animated_scene: true` and the inspector offers Speed. No external process is needed for this
subset.

Effects, particles, scripts, animated textures, pointer parallax, 3D and unsupported
materials still use the external renderer below. A partially supported scene is never
silently flattened with its missing parts omitted. Native compatibility is conservative
and has been checked on synthetic projects; real Wallpaper Engine projects still need
validation. Shared assets are found in Steam libraries, including Flatpak Steam, or through
`APOLLINE_WALL_ASSETS`. Set `APOLLINE_WALL_SCENE_RENDERER` to force an external renderer.

Building from source requires `apolline-scene` checked out beside this repository. The Arch
PKGBUILD fetches both repositories.


A project outside the native subset, or a web wallpaper, is drawn by [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine) (`linux-wallpaperengine-git` in the AUR), which runs as a process of its own on that monitor while ours steps aside. It reads the assets scenes are built from out of Wallpaper Engine's Steam install, so keep Wallpaper Engine installed. Video projects don't need it.

- It starts silent and ignoring the mouse, at the `fps` from `limits` when it starts, and `fit` becomes its scaling: `cover` fills, `contain` shows whole. Looks don't apply.
- Pause stops the process and resume continues it, so a paused scene costs nothing. It goes when the daemon goes, however the daemon goes.
- `ready` comes once it has lasted a moment, with the project's preview as the poster.
- If it stops on its own, the `error` carries its last words and the monitor goes back to what it showed before.
- Changes to and from an externally rendered project don't crossfade.
- `APOLLINE_WALL_SCENE_RENDERER` names another program that takes the same arguments; the tests use [`tests/scenes/stand-in.sh`](tests/scenes/stand-in.sh).
- `APOLLINE_WALL_SCENES=external` hands every scene to that program, even ones the daemon could draw itself. Either way the journal says why a scene went there.

## Favorites and rotation

Add a wallpaper to Favorites from its inspector. The Favorites filter and search can be
combined; favorites survive a library rescan and a restart.

Choose a monitor (or all monitors), filter the library to the wallpapers you want, then
choose **Rotate these wallpapers** (choose 1–60 minutes in the inspector). The daemon runs the playlist after the window
closes and restores it at login. Previous, next and stop are beside the playing monitors;
`apolline-wall next`, `previous` and `stop-playlist` also accept `--output NAME`.
Setting a wallpaper manually ends that monitor's own playlist.

An all-monitor playlist has a shared clock; pausing any participating monitor pauses that
clock, and stopping it stops the shared playlist. Choose individual monitors when independent
rotation is wanted. Paused or disconnected playlists have no timer; resuming starts a fresh
interval. The protocol's `playlist` command accepts `output`, absolute `sources`, `fit` and
`interval_secs` (10–86400). Empty `sources` stops a playlist. `step` accepts `output` and
`previous`; without an output it advances each active playlist once.

The runtime directory is private (0700) and checked for ownership and symlinks. Without
`XDG_RUNTIME_DIR`, clients use a private `apolline-wall-UID` directory under the temporary
folder. Clients still find a daemon left on the old `$XDG_RUNTIME_DIR/apolline-wall.sock` after an upgrade, and a new daemon won't start next to it.
IPC commands are capped at 2 MiB and subscriptions at 64; closing a client immediately
releases its subscription. Decoding has a two-result queue, so a busy event loop applies
backpressure instead of accumulating full-sized images.

In child mode stdout events are written off the render loop through a short queue. A parent
that reads slowly gets every event in order; while the queue is full only the newest status
waits, since statuses are snapshots. A broken output stream ends the child session, and the
parent restarts it and asks for the status again. A standalone daemon writes the same events to stdout, which the systemd journal keeps, and to every socket client; its stdout going away doesn't stop it.
External scene renderers require Linux 5.3+ with pidfd support: signals target a process
handle and each launch has a separate event identity. Native still-scene posters are capped
at 640×360, shader posters keep the full canvas for the lock screen, and PNG encoding runs on a worker; a stale completion never announces a newer
wallpaper as ready.

## Preview before applying

**Preview before applying** opens an isolated image preview with brightness, contrast,
saturation, mirror and fit controls. Adjustments stay inside the preview until **Set as
wallpaper**; Close or Esc discards them. Pictures are decoded with size limits, reduced to
1280×720 and adjusted on a worker with one render in flight. Video, shader and scene entries
show a labelled still preview, not animation. Entries whose renderer cannot apply our image
adjustments do not offer those controls. Full animated preview is still to come.

Image previews also support named look presets saved in the app configuration. Saving a preset is explicit and does not apply it to the desktop.

## Wallpaper-specific properties

Scene entries show supported controls declared by their author: checkboxes, sliders, colors,
text and dropdown options. Edits apply with **Set as wallpaper**, are remembered per wallpaper
and in daemon state, and Reset properties returns to author defaults. Loading the editor
reads only project metadata, not the scene package.

Property overrides travel in the `show` command's optional `properties` object. A native scene
resolves bindings before rendering. Scenes that need the external renderer reject explicit
property overrides instead of silently discarding them; an unmodified scene keeps its usual
fallback. Conditions, scripts, file pickers and unsupported property kinds are not executed.
The current static preview does not render these scene-specific edits; the desktop does when
Set as wallpaper succeeds. Playlists started from the app include saved scene properties and looks; unapplied editor changes are not included.

## Automatic playback rules

Settings → Automatic playback controls pause on screen lock, pause before sleep and battery
behavior. Lock and sleep pause are enabled by default; battery behavior defaults to Keep
running. Reduce shader and scene load caps FPS at 15 and resolution at 50%, while preserving any lower
user limit and the default 1080p pixel budget. Videos keep their normal decode quality;
choose Pause wallpapers to stop animation instead. These choices belong to the daemon and
survive a restart. `apolline-wall policy` prints them, for example
`apolline-wall policy --battery pause --lock true --sleep true` changes them.

System state follows logind (or a compatible elogind service) and UPower over D-Bus signals,
without periodic queries while connected. A lost service reconnects every five seconds;
Settings shows which integrations are available. The last observed state is kept during a
disconnection. A sleep delay inhibitor, when permitted, is released after the engine has
processed the pause, with a one-second maximum wait. Automatic pause reasons, per-monitor
manual pause and the global manual pause are independent. Unplugging AC or unlocking the
screen never clears a manual pause, and an automatic quality cap never overwrites user limits.

Lock detection needs a shell that updates logind's LockedHint. Shells that do not publish it
must send `apolline-wall session locked` and `apolline-wall session unlocked`, or the equivalent
child commands `{"session":{"locked":true}}` and `{"session":{"locked":false}}`. Send the
current state when starting the child and on each actual lock/unlock. The hook and logind
state are separate reasons; an unlocked logind hint does not cancel a shell-reported lock.
Apolline's lock integration needs this hook or LockedHint reporting; its source is maintained
separately and was not changed here. No D-Bus service is required for manual controls or the
shell hook.

Session, sleep and battery conditions are transient; only policy is saved. Policy changes
use `{"policy":{...}}`, with `pause_on_lock`, `pause_on_sleep`, `pause_on_fullscreen`, `battery` (keep/reduce/pause),
`battery_fps` and `battery_scale`. Status retains requested FPS/scale and includes a `power`
object with observed conditions and effective FPS. Each monitor reports effective `paused`,
`manual_paused` and an optional automatic `pause_reason`. A shared playlist clock stops while
any participating monitor is paused and restarts its interval after resuming.

The D-Bus interfaces follow [logind](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html)
and [UPower](https://upower.freedesktop.org/docs/UPower.html). Policy composition is unit-tested;
real sleep, lock and battery transitions still require hardware/session verification.


### Fullscreen windows on X11

By default, animation pauses independently on each monitor covered entirely by an EWMH
fullscreen window. Window-manager property, map and geometry events drive this rule; there
is no periodic scan. Leaving fullscreen, moving the window, minimizing it or changing
workspace reevaluates coverage. Manual pause and other automatic reasons remain independent.
Disable it in Settings or with `apolline-wall policy --fullscreen false`.

Desktop windows, the daemon's windows and the tracked external renderer's PID are excluded.
Hidden windows, windows on another workspace and explicitly translucent windows do not
pause playback. One fullscreen window must cover a whole monitor; partial windows are not
combined. Per-pixel transparency and compositor-specific rules cannot be inferred reliably
from EWMH, and native Wayland coverage is not implemented. This path follows the
[EWMH window properties](https://specifications.freedesktop.org/wm/latest/ar01s05.html);
geometry is unit-tested, but real window-manager behavior still needs session verification.


### External renderer diagnostics

The external renderer is watched through its pidfd as well as stderr. A descendant keeping
the stderr pipe open cannot delay crash detection and fallback. Closing stderr alone does
not count as process exit. Diagnostics retain at most three 4096-byte fragments, and each
read batch is bounded so continuous output cannot hide exit. This watcher sleeps in `poll`
without a periodic timeout. It tracks the renderer itself; it does not supervise all of its
descendant processes.


Library scans and external-renderer posters read `project.json` only as a regular file up to
8 MiB. Metadata's video and preview paths must resolve to files inside their project folder;
absolute paths, parent traversal and symlinks outside that folder are ignored. This is a
metadata path check, not a sandbox for an external renderer or protection against concurrent
filesystem changes by another process running as the same user.


Thumbnail creation is limited to two in-flight jobs shared by the local library and Workshop;
cached thumbnails do not wait for those slots. The blocking job owns its slot until it exits,
including when its awaiting UI task is cancelled. Thumbnail inputs are regular files up to
32 MiB; decoders apply a 16384-pixel side limit and 128 MiB allocation budget. Oversized or
broken previews are omitted. GIF thumbnails sample the middle of the first 48 frames using
two bounded passes, without retaining all those frames. The separate full picture preview
keeps its existing 256 MiB limit. These decoder limits do not represent a whole-process RSS
cap, and video thumbnails use an external ffmpeg process with a shared eight-second deadline across both seek attempts. A timed-out process is killed and reaped before its slot is released. Thumbnail files are published by atomic rename only after successful generation; failed or interrupted writes do not expose a partial cached file.

Video thumbnail output fits within 640×400 in either orientation; decoder, encoder and filter thread counts are limited to one. The deadline limits lifetime, not ffmpeg RSS.


### Failed playlist entries

A playlist skips an entry after a reported load/playback failure or external-renderer exit.
The next available entry is scheduled on the event loop, so a series of broken files cannot
recurse through the loader. Next and Previous also skip these entries. Duplicate paths are
removed while preserving order. One remaining healthy wallpaper stays on screen without a
repeating rotation timer; if every entry fails, rotation stops and reports how to recover.
Automatic pause still suspends advancement.

Failure marks belong to the running playlist: starting it again clears them, and choosing a
wallpaper manually permits an explicit retry. They do not survive a daemon restart or detect
a crash of the daemon itself. A shared playlist skips a failed source for all participating
monitors. An external scene's fallback cannot silently detach a monitor from that shared
playlist. Poster-writing errors do not mark an otherwise playable wallpaper as failed.


Shader source loading requires a regular UTF-8 file up to 1 MiB; reads of growing files are
bounded as well. A runtime drawing error stops the shader, releases its GL resources and
reports the failing source once. Playlist recovery can then choose the next entry; manual
selection retries explicitly. Shader frame timers carry the displayed wallpaper's generation,
so a callback from a replaced shader cannot advance its replacement. Source-size limits are
not a sandbox for GLSL compilation or GPU drivers.


### Playlist controls and status

Each monitor's playback card shows its playlist position, skipped entries and whether the
clock is paused or the list has no playable entries. Monitor cards scroll horizontally;
Pause all stays outside that scroll area. Previous, Next and Stop are also available from the
tray without opening the app. A shared playlist is labelled explicitly, including its Stop
action, because changing its position or stopping it affects every participating monitor.
Steps are disabled when there is no different playable entry; stopping remains available.
An exhausted list explains that a manual selection or starting the list again permits retry.

The daemon publishes this information in each monitor's optional `playlist_status` object,
with `position` (one-based), `total`, `skipped`, `shared`, `suspended` and `can_step`. The older
`playlist` flag remains available. Clients reading older status messages retain the original
controls without detailed counts. `apolline-wall status` also prints the playlist summary.
Failure status is published immediately, including when every entry has failed and no further
rotation event will arrive. These UI changes are compiled and unit-tested; their actual
layout and tray-host rendering still need graphical verification.


### Per-wallpaper settings in playlists

Starting rotation in the app captures the saved look and scene property overrides for each
included wallpaper. Every transition and daemon restart uses that snapshot; settings do not
leak between entries. Apply editor changes before starting rotation. Later app configuration
changes do not rewrite an already-running list; start it again to capture new saved values.
The playlist's fit setting still applies to every entry. Random wallpaper from the tray also
uses that wallpaper's saved properties and look.

The optional `playlist.settings` object maps source paths to `{ "properties": {...}, "look":
{...} }`. Only listed paths are accepted. Property counts and look ranges are checked before
accepting the command; the existing 2 MiB command limit also bounds the full snapshot. Schema
validation remains part of native scene loading. Explicit properties on unsupported scenes
are rejected and skipped by playlist recovery, rather than silently discarded by the external
renderer. Old playlists without `settings` retain author defaults and a plain look. Saved
snapshots are included in daemon state and survive restart; temporary failure marks do not.


### Editing a monitor during shared rotation

Changing one monitor's look keeps it in the shared playlist. Its neighbours retain their
own visual settings, and automatic pause still accounts for that monitor. The current edit
is tied to the current wallpaper: when the shared list advances, the next entry uses its
playlist snapshot instead of inheriting the previous entry's look. A manual wallpaper choice
still excludes that monitor from shared rotation, even when the chosen file is the same.

Reverting to the default look removes a derived monitor assignment. Applied edits retain
that assignment's inherited-source marker in daemon state, so restarting does not turn a
look adjustment into an independent wallpaper choice. Drafts do not save that marker or their
look. Reloading a draft after a resize also cannot promote its look into the committed
last-good fallback. Legacy saved assignments without the marker retain their previous
explicit-assignment meaning; their original intent cannot be inferred safely.

This separates source selection and visual edits; it does not change libmpv's shared-player
architecture. Monitors using the same video still share its playback clock and speed.


### Shader timing and delayed video readiness

Shader animation accumulates time in f64 on the CPU; GLES uniforms remain f32, so very large
iTime values still have GPU float precision limits. The logical iTimeDelta follows playback
speed consistently with iTime. Gaps are capped at the larger of 250 ms and two requested
frame intervals, avoiding a catch-up jump after a long interruption while preserving normal
1 FPS timing and scheduling jitter. Explicit pause and paused manual redraws advance no animation time. iFrame
saturates at its signed-integer maximum instead of overflowing.

Pending shader timers are removed on pause, replacement and forced redraw. Changing FPS
reschedules a waiting shader instead of waiting out the previous slow deadline. These are
shader clock and scheduler changes, not native animation support for Wallpaper Engine scenes.

Video poster readiness is tied to the monitor's displayed generation. Reattachments coalesce
per monitor, and a late poster cannot announce a wallpaper that monitor has already left.
Refresh also releases video players whose pending load was cancelled before the first frame;
players still shown or requested by another monitor remain shared.

Native still scenes now accept apolline-scene's supported static fragment-only materials, including up to eight shared RGBA-decoded texture inputs and opacity-mask shaders. A GPU compilation/preparation error with explicit scene property edits keeps the previous wallpaper; those edits are not handed to the external renderer, including failures that occur after CPU preparation. Unsupported unmodified scenes retain the external fallback. Real Workshop materials and GLES runtime behavior still need validation.


Native fragment animation uses g_Time/g_Frametime with static source textures. It shares the
shader scheduler, limits, manual/automatic pause and crossfades. Status marks it with
native_scene and animated_scene; the inspector offers Speed once it is active. Compiled
materials with no active time uniform keep the still path and have no animation timer.
Scene posters use a bounded 640×360 readback and worker PNG encoding; decoded source pixels
are released after GPU preparation. Effect chains, particles, scripts, custom vertices and
animated textures still require the external renderer. Linux compilation/unit tests and
synthetic macOS GL snapshots cover this path; Linux GLES runtime validation is pending.


Shadertoy posters now use the same bounded readback/PNG worker as native scenes. Completion
messages carry metadata, not decoded pixels. PNGs are encoded to an exclusive temporary file,
flushed, then renamed; a failed write preserves the previous cached PNG. A preview-cache error
still reports the running wallpaper as ready and emits a separate warning without a source
failure. Stale generations remain ignored. Updating an active native scene's parameters keeps
its animation phase and excludes preparation delay from its clock; changing sources starts a
new clock. Native frames do not compute Shadertoy date uniforms or redraw for unused accent
changes. GL draw errors remain playback failures, separate from preview-cache errors.

## Trying it

```sh
cargo build --release
tools/headless.sh shots/   # Linux: under a throwaway headless sway, a screenshot per step
```

`tools/headless.sh` needs sway and grim; it shows the pictures in `tests/pictures`, crossfades, pauses, and checks that a missing file and an unknown command come back as errors. With ffmpeg and mpv installed it also plays a generated clip, checks that it moves and that a paused one doesn't, and fades from it back to a picture. Then it runs `tests/shaders/hills.frag`, checks that it moves, that a paused one doesn't and that a new accent shows anyway, and fades back to a picture. With swaybg installed a stand-in plays linux-wallpaperengine's part: a scene shows, its process stops while paused and is gone after the next wallpaper, and one that crashes leaves the desktop as it was. With apolline-scene cloned next to this repository it also draws its synthetic scene in the daemon itself, and checks that it shows and that nothing went to the external renderer.

## Next

Effect chains and animation in the native scene renderer, real Workshop corpus checks, GNOME/KDE/X11 validation, stable monitor identities and packages for other distributions. See [`docs/ROADMAP.md`](docs/ROADMAP.md).

## License

MIT or Apache-2.0, at your option.
