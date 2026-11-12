<h1 align="center">apolline-wall</h1>

<p align="center"><b>Live wallpapers for Linux: pictures, videos, shaders and Wallpaper Engine scenes, on every monitor.</b><br>
Workshop scenes drawn natively by its own renderer, with particles, effects and scripts. Crossfades between wallpapers, one run for a wallpaper shown on several monitors, and nothing spent while nothing shows. A daemon, a CLI and an app to pick them; the wallpaper process of Apolline.</p>

<p align="center">
<a href="https://github.com/Pisosenie/apolline-wall/actions/workflows/packages.yml"><img alt="CI" src="https://github.com/Pisosenie/apolline-wall/actions/workflows/packages.yml/badge.svg"></a>
<img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20or%20Apache--2.0-5dd39e">
<img alt="Runs on Wayland and X11" src="https://img.shields.io/badge/runs%20on-Wayland%20%C2%B7%20X11-c084fc">
</p>

<p align="center"><a href="#quick-start">Quick start</a> · <a href="#installing">Install</a> · <a href="#how-to">How to</a> · <a href="#troubleshooting">Troubleshooting</a> · <a href="#talking-to-it">Protocol</a> · <a href="#what-is-stable">What is stable</a></p>

apolline-wall puts moving wallpapers on a Linux desktop the way Wallpaper Engine does on Windows, and reads Wallpaper Engine's own Workshop items to do it. Videos play through libmpv, Shadertoy shaders run as they are, and Wallpaper Engine scenes are drawn by [apolline-scene](https://github.com/Pisosenie/apolline-scene), a renderer written for this, so a scene gets the same crossfades, looks, pause and frame limits as everything else. It draws on a layer-shell surface per monitor with GLES at each monitor's physical resolution, so it runs on niri, Hyprland, sway, river, labwc, Wayfire and COSMIC; on KDE Plasma a wallpaper plugin shows it under Plasma's icons and widgets; on GNOME its Shell extension puts a window per monitor behind everything, and in X11 sessions it draws in a desktop window per monitor. [Apolline](https://github.com/Pisosenie/apolline) runs it as a child process, so video decoding never ends up inside the shell. Arch first; [`docs/ROADMAP.md`](docs/ROADMAP.md) says what comes next.

## At a glance

| | |
|---|---|
| **Pictures** | any common format, crossfaded, nothing drawn between changes |
| **Videos** | mp4, webm, mkv, mov, gif and the like, muted and looping through libmpv with hardware decoding; one decode for every monitor |
| **Shaders** | single-pass Shadertoy `.frag`/`.glsl`, with the shell's accent as `iAccent` |
| **Wallpaper Engine** | videos and scenes from your Workshop subscriptions; scenes natively, web wallpapers and the rest through [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine) |
| **Per monitor** | its own wallpaper, fit (cover, contain, blur), look and playlist, following the monitor across ports |
| **When unseen** | paused on lock, before sleep, optionally on battery; no timers or decoding while nothing is visible |
| **Desktops** | niri, Hyprland, sway, river, labwc, Wayfire, COSMIC; KDE Plasma 6, GNOME 45–49 and X11 sessions written but not yet run on real hardware |
| **Ways in** | `apolline-wall` CLI, the `apolline-wallpapers` app and tray, JSON lines over a socket, or Apolline's Settings |

## Quick start

On Arch:

```sh
git clone https://github.com/Pisosenie/apolline-wall
cd apolline-wall/packaging/arch && makepkg -si
```

Stop whatever draws your wallpaper now (hyprpaper, swww, swaybg), start the daemon, and set something:

```sh
systemctl --user enable --now apolline-wall.service      # or a line in your compositor's config, see below
apolline-wall set ~/Videos/rain.mp4                       # every monitor
apolline-wall set ~/.local/share/Steam/steamapps/workshop/content/431960/1234567890   # a Workshop scene
apolline-wallpapers                                       # or pick in the app
```

Under Apolline there is nothing to start: pick a live wallpaper in Settings → Appearance.

## Wallpaper Engine scenes, natively

A scene is a Workshop item made of layers, materials, shaders, particles, effects and scripts. apolline-wall prepares each one with [apolline-scene](https://github.com/Pisosenie/apolline-scene), its own clean-room renderer on OpenGL or GLES, and draws it in the daemon's GL context next to pictures, videos and shaders. That covers image layers with 32 blend modes, materials with their vertex and fragment shaders, multi-pass and composed effects, bloom, particles (emitters, turbulence, control points, children, rope trails), puppet warp, timelines, text layers, sprite sheets, camera parallax and shake following the pointer, the time of day, and the author's properties: checkboxes, sliders, colors, dropdowns and text. Scene scripts (SceneScript) run in a QuickJS sandbox with no filesystem and no network.

On an Arch machine with an NVIDIA RTX 4070 Ti, all 27 scenes that prepare natively in a real Workshop library draw, and a frame of each matches the project's own preview. A scene that uses something not done yet (video textures, 3D layers and cameras, HDR bloom, some particle features; [the full list](https://github.com/Pisosenie/apolline-scene#what-it-draws)) is refused up front with the reason, never drawn with parts left out, and goes to linux-wallpaperengine instead if it is installed. Web wallpapers always do. Application wallpapers are Windows programs and say so. Scenes that move to music hear what's playing through `parec` (libpulse), which is new and not yet checked on hardware.

## One scene, many monitors

A moving scene shown on several monitors runs once, into a canvas of the scene's own shape, and each monitor shows its part of it: cover, contain, or contained with the edges filled by a blurred copy. Particles, scripts and effects cost the same however many monitors there are, as one decode already feeds every monitor showing the same video. The pointer for parallax comes from the monitor it was last over. On two monitors in a test this took a quarter off the CPU time and GPU use from 46 % to 32 %.

## Free when unseen

A frame is drawn only when the compositor asks for one, so a wallpaper covered by a fullscreen window costs nothing: in a test VM a 1080p clip went from about seven cores to under one. Static pictures draw nothing between changes and keep no timers. Animation pauses on screen lock and before sleep, and on battery it can keep running, cap shaders and scenes at 15 fps and half resolution, or pause. On X11 a monitor covered by a fullscreen window pauses on its own. A manual pause and the automatic ones are kept apart, so unlocking the screen never resumes something you paused.

## The Wallpapers app

`apolline-wallpapers` is a window to browse and set wallpapers, in Apolline's look, on any desktop. It talks to the daemon over the same socket as the CLI and starts the daemon if it isn't running.

- **Library.** Pictures, videos and shaders from `~/Pictures/Wallpapers`, `~/Videos/Wallpapers`, `~/.local/share/apolline-wall/wallpapers` and any folder you add in Settings, plus everything Wallpaper Engine has on this machine: Workshop subscriptions and its own projects, from every Steam library, native client and Flatpak alike. Filters by kind, search and Favorites.
- **Steam Workshop.** Wallpaper Engine's Workshop, browsed without an account: videos, scenes, everything or collections; trending, newest or most subscribed; search. **Subscribe in Steam** hands an item to the Steam client, and **Download** goes through `steamcmd` without it.
- **Inspector.** Every monitor or one; fill, show whole, or show whole with blurred edges; brightness, contrast, saturation, mirror and speed as a live draft on the desktop with **Apply** and **Revert**; the scene's own properties with presets.
- **Rotation.** Filter the library and **Rotate these wallpapers** every 1–60 minutes, per monitor or on all of them.
- **Tray.** `apolline-wallpapers --tray`: pause, resume, previous, next and a random wallpaper per monitor.
- **Report a problem.** "Not working? Report it" writes a plain-text report for an issue.

Details are in [The app in full](#the-app-in-full).

## Inside Apolline

[Apolline](https://github.com/Pisosenie/apolline) runs apolline-wall as a child process and talks JSON lines with it over stdin and stdout; the shell decides what each monitor shows and draws its clock and widgets on top. You pick live wallpapers in Apolline's Settings → Appearance, where scenes and videos from every Steam library are marked with ▶. Wallpapers pause while the screen is locked and in game and performance modes, so a game gets the whole GPU. The poster frame apolline-wall sends back feeds Apolline's accent and lock screen, and stands in if apolline-wall can't draw. The `apolline-wallpapers` app works alongside: it talks to the same daemon over its control socket, `$XDG_RUNTIME_DIR/apolline-wall/control.sock`.

## Safe with anyone's files

A Workshop item is someone else's file, so it gets no more than it needs.

- Videos play through mpv restricted to local files: ffmpeg's demuxers only, a protocol whitelist of `file`, no playlists, EDLs or references to other files, no ytdl, no scripts. A "video" that is really a playlist can't make it fetch a URL.
- Scene scripts run in a QuickJS runtime per renderer with only the scene API: no filesystem, no network, a 64 MiB heap and 50 ms a frame.
- `project.json` is read only as a regular file up to 8 MiB, and its video and preview paths must stay inside the project folder.
- Thumbnails, shader sources and IPC commands all have size limits (see [Limits and safeguards](#limits-and-safeguards)).
- Steam credentials are never stored: the password and Steam Guard code go only to steamcmd's prompt.

## Installing

**Arch Linux.** `packaging/arch` builds `apolline-wall-git` from the newest commit:

```sh
git clone https://github.com/Pisosenie/apolline-wall
cd apolline-wall/packaging/arch && makepkg -si
```

It fetches apolline-scene at the commit it pins and builds without LTO on purpose (gcc's LTO objects from ring's and QuickJS's C parts can't be read by Rust's linker). It installs `apolline-wall`, `apolline-wallpapers`, the systemd user unit, a desktop entry, the GNOME Shell extension and a sample shader. Optional dependencies, by what they add:

| Package | For |
|---|---|
| `mpv` | video wallpapers |
| `ffmpeg` | video thumbnails in the app |
| `linux-wallpaperengine-git` | scenes apolline-scene can't draw yet, and web wallpapers |
| `steamcmd` | Workshop downloads without the Steam client |
| `libx11` | X11 sessions, and GNOME without its extension through Xwayland |
| `vulkan-icd-loader` | faster drawing for the app |
| `noto-fonts-cjk` | Chinese, Japanese and Korean titles in the Workshop |

**Debian 12, Ubuntu 22.04 and newer**, amd64 or arm64: the `.deb` attached to each release, built and tested on Ubuntu by CI. It leaves out the Plasma plugin: Ubuntu 22.04's PipeWire is older than the plugin needs, and has no Plasma 6 to plug into; build from source with `cargo build --release` on a newer system for it.

```sh
sudo apt install ./apolline-wall_*.deb
```

**From source**, with a current stable Rust and [apolline-scene](https://github.com/Pisosenie/apolline-scene) checked out beside this repository:

```sh
git clone https://github.com/Pisosenie/apolline-scene
git clone https://github.com/Pisosenie/apolline-wall
cd apolline-wall && cargo build --release
```

### Running it on its own

Under Apolline the shell starts it. Anywhere else, run it as a daemon: it restores what each monitor showed and keeps running after you close the app. The package ships a systemd user unit tied to `graphical-session.target`:

```sh
systemctl --user enable --now apolline-wall.service
```

or start it from your compositor:

| Session | Line |
|---|---|
| niri | `spawn-at-startup "apolline-wall" "daemon"` in `config.kdl` |
| Hyprland | `exec-once = apolline-wall daemon` |
| sway | `exec apolline-wall daemon` |
| KDE Plasma | the systemd line above, then pick **apolline-wall** under Desktop and Wallpaper → Wallpaper type (see below) |
| GNOME | `gnome-extensions enable apolline-wall@pisosenie.github.io`, log out and back in, then the systemd line above |

A second instance finds the socket alive and leaves, so starting it twice is harmless.

### KDE Plasma

Plasma 6 draws its own desktop, icons and widgets included, as an opaque surface on layer-shell's background layer, so a wallpaper there would never show. The package installs a Plasma wallpaper plugin instead: right-click the desktop → Desktop and Wallpaper → Wallpaper type → **apolline-wall**, on each monitor. The plugin asks the daemon for its monitor (`apolline-wall feed <connector>`); the daemon then draws that monitor offscreen and sends its frames to a PipeWire stream, which the plugin plays inside Plasma's desktop, under its icons and widgets. Everything else, from `apolline-wall set` to the Wallpapers app, works as on any other desktop.

Until a monitor's plugin asks, and on a monitor that never used the plugin, the daemon draws on the bottom layer: over Plasma's desktop, under windows and panels, with clicks passing through to the desktop, but covering its icons and widgets.

A monitor whose desktop a maximized or fullscreen window hides pauses, the way Plasma's own animated wallpapers do; the plugin tells the daemon, and the "behind fullscreen windows" rule turns it off. Limits for now: frames are read back from the GPU and copied, which costs more than drawing on a surface of our own; scenes that follow the pointer hold still; and scenes left to linux-wallpaperengine don't show through the plugin. Needs `kpipewire` (installed with Plasma). It has not been run on Plasma yet.

### GNOME

GNOME has no background layer for other programs, so the daemon opens an ordinary window per monitor (app id `apolline-wall`, titled `apolline-wall:<connector>`) and the Shell extension in `gnome/` places each on its monitor, behind every window, on every workspace, and keeps it out of the overview and Alt+Tab. The daemon opens those windows only while the extension is enabled, so it never covers your desktop. It targets GNOME 45–49 and has not been run on GNOME yet.

## How to

### Set a wallpaper

```sh
apolline-wall set ~/Pictures/dunes.jpg                  # every monitor
apolline-wall set ~/Videos/rain.mp4 --output DP-1       # one monitor; `apolline-wall status` lists their names
apolline-wall set ~/shaders/hills.frag --transition 0   # no crossfade
apolline-wall set ~/Videos/wide.mp4 --fit blur          # whole, edges filled with a blurred copy; also cover, contain
```

`set` waits until the wallpaper shows and says why if it can't. A sample shader is installed at `/usr/share/apolline-wall/wallpapers/hills.frag`.

### Use Wallpaper Engine's Workshop

1. Buy Wallpaper Engine on Steam and install it with Proton (Steam → its Properties → Compatibility). Scenes need its `assets` folder, so keep it installed.
2. Subscribe to wallpapers in the Workshop, in Steam or from the app's **Steam Workshop** tab.
3. They show up in the app's library, in Apolline's Settings → Appearance, and as folders named by ID under `steamapps/workshop/content/431960` in each Steam library:

```sh
apolline-wall set ~/.local/share/Steam/steamapps/workshop/content/431960/1234567890
```

Without the Steam client, sign in once under **Steam account** in the app's Settings and use **Download**; items land in `~/.local/share/apolline-wall/steamcmd`. Assets in an unusual place can be pointed at with `APOLLINE_WALL_ASSETS`.

### Limit how hard it works

```sh
apolline-wall limits --fps 20 --scale 0.5    # shaders and scenes: frames per second (1–240) and share of resolution (0.1–1)
apolline-wall policy --battery reduce        # on battery: keep, reduce or pause
apolline-wall policy                         # print the current rules
```

The default is 30 fps at about a 1080p screen's worth of pixels.

### Pause and resume

```sh
apolline-wall pause                    # everything
apolline-wall pause --output HDMI-A-1  # one monitor
apolline-wall resume
```

### Rotate through a playlist

In the app, filter the library to what you want, pick a monitor or all of them, choose an interval and **Rotate these wallpapers**. The daemon keeps rotating after the window closes and after a login. From a terminal:

```sh
apolline-wall next            # also previous, stop-playlist; each takes --output NAME
```

Or play a folder: whatever pictures, videos, shaders and Wallpaper Engine project folders it holds, in name order. It is read again at every step, so files you add join and files you remove leave, and the one showing keeps its place:

```sh
apolline-wall playlist ~/Pictures/Walls --every 600 --output DP-1   # --fit too; every 300 s by default
```

To start a list of your own without the app, send the `playlist` command to the socket:

```sh
echo '{"playlist":{"sources":["/home/me/Videos/a.mp4","/home/me/Pictures/b.jpg"],"interval_secs":600}}' \
  | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/apolline-wall/control.sock
```

### Use it under niri, Hyprland or sway without Apolline

Add the daemon to your startup (or use the systemd unit), and the tray if you want one:

```kdl
// niri, config.kdl
spawn-at-startup "apolline-wall" "daemon"
spawn-at-startup "apolline-wallpapers" "--tray"
```

```ini
# Hyprland
exec-once = apolline-wall daemon
exec-once = apolline-wallpapers --tray
```

```
# sway
exec apolline-wall daemon
exec apolline-wallpapers --tray
```

Remove hyprpaper, swww or swaybg lines first: two programs on the background layer fight over it. If your shell locks the screen without updating logind's LockedHint, have it run `apolline-wall session locked` and `apolline-wall session unlocked` so wallpapers pause.

## Troubleshooting

- **What does it show, and is it paused?** `apolline-wall status`: each monitor's wallpaper, pause state and reason, playlist position, limits and power state.
- **Why didn't it show?** `set` prints the reason. The daemon writes every event, errors included, to stdout, which the journal keeps: `journalctl --user -u apolline-wall -f`. Under Apolline the events go to the shell. The journal also says why a scene went to linux-wallpaperengine.
- **A scene looks wrong.** `APOLLINE_WALL_SCENES=external` hands every scene to linux-wallpaperengine, to compare. Without either renderer able to draw it, the app says what to install.
- **A wallpaper took the daemon down.** One that does so twice in a row is skipped at the next start with an error saying so, until you set it again.
- **Report it.** "Not working? Report it" in the app's inspector saves a plain-text report to Downloads: the wallpaper and what it is, the distribution and session, which of mpv, ffmpeg, linux-wallpaperengine, steamcmd and Steam are installed, what the daemon shows and its last errors, with your home folder and user name taken out. Attach it to an issue.

---

The rest is reference.

- [The app in full](#the-app-in-full)
- [Monitors and state](#monitors-and-state)
- [Talking to it](#talking-to-it)
- [Videos](#videos) · [Shaders](#shaders) · [Scenes](#scenes)
- [Playlists](#playlists)
- [Preview, looks and properties](#preview-looks-and-properties)
- [Automatic playback rules](#automatic-playback-rules)
- [Limits and safeguards](#limits-and-safeguards)
- [Trying it](#trying-it)

## The app in full

- **Library.** Pictures, videos and shaders from the folders above and any you add in Settings, and Wallpaper Engine's Workshop subscriptions and its own projects. Its videos and scenes play here; anything that needs linux-wallpaperengine says what to install if it's missing. Application wallpapers say they are Windows programs.
- **Steam Workshop.** Collections open to what they hold, and **More from the author** lists the rest of an author's work. Items rated Questionable show only if you turn them on in Settings; Mature ones never do. Collections have no rating, so one shows only if its first four wallpapers pass. `apolline-wallpapers --page workshop` opens on this tab (`--page settings` on Settings).
  - With the Steam client, **Subscribe in Steam** opens the item there; the app watches for the download only while one is pending, and the item appears in the library ready to set.
  - Without it, **Download** goes through `steamcmd` with a Steam account that owns Wallpaper Engine. steamcmd asks for the password and the Steam Guard code; the app shows its question and hands your answer straight to it, and keeps only the account name. Signing in checks that the account owns Wallpaper Engine. Downloads go one at a time into a staging folder and move into place when done; each time the tab opens the app asks the Workshop which have a newer version, for an **Update** button.
- **Favorites.** From the inspector; the filter combines with search, and favorites survive a rescan and a restart.
- **Now playing.** A card per monitor with what it shows, pause, and its playlist's position with previous, next and stop.
- **Tray.** A StatusNotifierItem that waits for the bar at login. Settings can start it at login through XDG autostart; on niri or Hyprland add the command to your startup. Every client gets a fresh status after any change, so the tray, the window and the shell agree.

Thumbnails are cached in `~/.cache/apolline-wall`: videos get theirs from ffmpeg, shaders once the daemon has shown them, animated Workshop previews a frame from their middle. Workshop previews are kept under 150 MB, oldest out first.

## Monitors and state

Each monitor's wallpaper follows the monitor, not the port: plug it into another one, or into a dock, and it comes back with the same wallpaper, look and playlist. The daemon recognises a monitor by what the compositor calls it on Wayland (its description, usually with a serial) and by its EDID on X11; two identical monitors without serials can't be told apart and don't move.

What each monitor shows, the limits and the rules are kept in `~/.config/apolline-wall/state.toml`, so a login comes back to the same desktop. A wallpaper that fails is not written there; the last one that showed stays. Under Apolline the shell decides instead.

## Talking to it

The daemon listens on `$XDG_RUNTIME_DIR/apolline-wall/control.sock`: one JSON command per line in, one event per line out. Apolline starts it without arguments and talks over stdin and stdout instead; then it exits when stdin closes, so it never outlives the shell. The CLI, the app and Apolline all speak the same lines.

| Command | Does |
|---|---|
| `{"show":{"source":"/abs/path.jpg"}}` | every monitor; a folder is a Wallpaper Engine project; also `"output":"DP-1"`, `"fit":"cover"\|"contain"\|"blur"`, `"transition_ms":400`, `"look"`, and a scene's `"properties"` |
| `{"pause":{"paused":true}}` | no animation, videos and shaders stop, changes land at once; `"output"` for one monitor |
| `{"limits":{"fps":30,"scale":0.5}}` | frames per second (1–240) and share of resolution (0.1–1) for shaders and scenes; either can be left out |
| `{"uniforms":{"accent":"#86bdf0"}}` | the shell's accent, which shaders read as `iAccent` |
| `{"adjust":{"look":{"brightness":-0.2,"contrast":1,"saturation":1,"flip":false,"speed":1},"draft":true}}` | the look of what is shown now; fields left out keep their defaults; `"output"` for one monitor; a draft shows but isn't remembered |
| `{"playlist":{"sources":[...],"interval_secs":600}}` | rotate through absolute paths every 10–86400 s; also `"output"`, `"fit"`, `"settings"`; or `"folder"` instead of `"sources"` for what a folder holds, read again at every step; empty `sources` and no folder stops it |
| `{"step":{"previous":false}}` | next or previous entry; `"output"` for one monitor, otherwise every active playlist once |
| `{"policy":{...}}` | `pause_on_lock`, `pause_on_sleep`, `pause_on_fullscreen`, `battery` (`keep`/`reduce`/`pause`), `battery_fps`, `battery_scale` |
| `{"session":{"locked":true}}` | the shell's lock state, for shells that don't update logind |
| `{"feed":{"output":"DP-1"}}` | Plasma's wallpaper plugin asking for a monitor's frames as a PipeWire stream instead of a surface of its own, until the daemon restarts |
| `{"status":{}}` | answered to whoever asked with a `status` event |

Events go to stdout and to every socket client:

- `{"ready":{"output","source","poster"}}` once a monitor shows the new wallpaper; `poster` is a still image of it, for the shell's accent and the lock screen.
- `{"error":{"output","source","message"}}` when a file can't be shown or a line isn't a command. A file that failed is not retried until the next `show`.
- `{"status":{...}}`: each monitor's `name`, `source`, effective `paused`, `manual_paused`, an optional `pause_reason`, `native_scene` and `animated_scene` for scenes, `feed` (its PipeWire `node`, and `stream`, which unlike node ids is never reused) while Plasma's plugin shows it, and `playlist_status` (`position`, `total`, `skipped`, `shared`, `suspended`, `can_step`); the requested `fps` and `scale`, and a `power` object with what logind and UPower report and the effective fps. After `pause`, `limits`, `uniforms` and `adjust` every client gets a fresh one.

## Videos

A `source` ending in `.mp4`, `.webm`, `.mkv`, `.mov`, `.gif` and the like plays muted and looping through libmpv, with hardware decoding where the driver offers it. libmpv is opened only when the first video is shown, so pictures work without mpv installed, and a video then answers with an error saying so.

- One decode feeds every monitor showing the same file, at the size the most demanding monitor needs and never above the video's own. Monitors showing the same video share its clock and speed.
- A frame is rendered only when a monitor showing the video is ready for one, so decoding stops while a fullscreen window covers it.
- mpv runs in its simplest rendering mode: no HDR tone mapping or fancy scalers, which a wallpaper doesn't need and a battery would pay for.
- The poster is read back from the video's own canvas and written to `$XDG_RUNTIME_DIR/apolline-wall/poster-*.png` before `ready` is sent. A poster that can't be saved is a warning, not a failed video.

## Shaders

A `source` ending in `.frag` or `.glsl` is a fragment shader written for [Shadertoy](https://www.shadertoy.com): a `mainImage(out vec4 fragColor, in vec2 fragCoord)` with `iTime`, `iTimeDelta`, `iFrame`, `iFrameRate`, `iResolution`, `iDate` and `iChannel0`–`iChannel3`, which all hold a tiling noise texture since there is nowhere to pick another yet. `iAccent` is the shell's accent as RGB in 0–1, so a wallpaper can follow the mode you're in. Most single-pass shaders paste in as they are; multi-pass buffers, sound and keyboard input are not supported. Mind their licenses: Shadertoy's default is CC BY-NC-SA. [`tests/shaders/hills.frag`](tests/shaders/hills.frag) is one of ours.

- A shader draws into a canvas of its own per monitor, about a 1080p screen's worth of pixels unless `limits` says otherwise, stretched over the monitor, at most `fps` times a second and only when the compositor asks for a frame.
- A paused shader keeps its clock still and resumes where it stopped; a new accent shows at once, on paused shaders too.
- A shader that doesn't compile comes back as an `error` with the compiler's message, its line numbers pointing into your file. A drawing error at run time stops it, frees its GL resources and reports it once.
- The poster is its first frame.

## Scenes

The daemon prepares each scene with apolline-scene on its loading thread, off the event loop, before any GL work. Shared assets are found in every Steam library, Flatpak Steam included, or through `APOLLINE_WALL_ASSETS`.

- A scene with nothing moving is drawn once at the monitor's resolution and keeps no timer.
- A moving scene shares the shader scheduler: `fps` and `scale` limits, manual and automatic pause, crossfades. On several monitors it runs once into a shared canvas, sized for the most demanding monitor within the pixels separate canvases would have taken.
- Crossfades, cover, contain and blur, brightness, contrast, saturation, mirror and speed apply. Status marks these monitors with `native_scene: true`, and moving ones with `animated_scene: true`.
- Scenes that follow the pointer draw only when it moves them. Wayland reports the pointer when it is over the desktop; X11 is asked only while such a scene shows.
- The author's properties are resolved before drawing. Changing them keeps the scene's animation phase.
- Each scene may use up to 1 GiB of textures and render targets. Posters are read back at most 640×360.

A project apolline-scene refuses, and every web wallpaper, goes to [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine) (`linux-wallpaperengine-git` in the AUR), which runs as a process of its own on that monitor while ours steps aside. Video projects don't need it.

- It starts silent and ignoring the mouse, at the `fps` from `limits` when it starts (a new limit reaches it the next time it starts), and `fit` becomes its scaling: `cover` fills, `contain` shows whole. Looks and speed don't apply, and changes to and from it don't crossfade.
- Pause stops the process and resume continues it. It goes when the daemon goes, however the daemon goes.
- `ready` comes once it has lasted a moment, with the project's preview as the poster. If it stops on its own, the `error` carries its last words and the monitor goes back to what it showed before.
- A scene with edited properties is never handed to it, since it would ignore them: the edit is refused and the previous wallpaper stays. An unedited scene falls back as usual.
- `APOLLINE_WALL_SCENE_RENDERER` names another program that takes the same arguments; the tests use [`tests/scenes/stand-in.sh`](tests/scenes/stand-in.sh). `APOLLINE_WALL_SCENES=external` sends every scene there, even ones the daemon could draw.
- It needs Linux 5.3 or later: the process is watched through a pidfd as well as its stderr, so a descendant holding stderr open can't delay noticing that it died. Its diagnostics keep at most three 4096-byte fragments.

## Playlists

A playlist rotates through wallpapers on one monitor, or on all of them on a shared clock. It persists across restarts. Setting a wallpaper by hand ends that monitor's own playlist, and excludes it from a shared one even if the file is the same.

- **Shared clock.** Pausing any participating monitor pauses the clock, and stopping it stops it for all. Paused or disconnected playlists keep no timer; resuming starts a fresh interval. Pick monitors one by one for independent rotation.
- **Failed entries.** An entry that fails to load or play, or whose external renderer exits, is skipped for the life of the running playlist, in Next and Previous too, and for every monitor of a shared one. One healthy entry left stays on screen without a timer; if every entry fails, rotation stops and says how to recover. Starting the list again or choosing a wallpaper by hand allows a retry. Duplicate paths are dropped, order kept.
- **Per-wallpaper settings.** Starting rotation in the app captures each wallpaper's saved look and scene properties into the playlist's `settings` (paths mapped to `{"properties":{...},"look":{...}}`), and every transition and restart uses that snapshot. Apply editor changes before starting; start it again to pick up later ones. The playlist's fit applies to every entry. The tray's random wallpaper uses saved settings too.
- **Editing during shared rotation.** Changing one monitor's look keeps it in the shared playlist, and the edit belongs to the current entry: the next one uses its own snapshot.
- **Controls.** Each playback card and the tray show position, skipped entries, whether the clock is paused, and previous, next and stop; a shared playlist is labelled as such, since its controls affect every monitor in it. `apolline-wall status` prints the same summary.

## Preview, looks and properties

**Preview before applying** opens an isolated picture preview with brightness, contrast, saturation, mirror and fit. Nothing reaches the desktop until **Set as wallpaper**; Close or Esc discards it. Pictures are decoded with size limits and reduced to 1280×720 on a worker. Video, shader and scene entries show a labelled still; full animated preview is still to come. Look presets can be saved by name, which does not apply them.

Looks are drawn in the final shader, so they cost nothing: brightness, contrast, saturation and mirror for everything apolline-wall draws itself, and speed for videos, shaders and moving scenes. In the inspector each change shows on the chosen monitor at once as a draft; **Apply** keeps it, **Revert** goes back, and each wallpaper remembers its look per monitor.

Scene entries show the controls their author declared: checkboxes, sliders, colors, text and dropdowns. Edits apply with **Set as wallpaper**, are remembered per wallpaper and in daemon state, and **Reset properties** returns to the author's defaults. Values can be saved as named presets and copied to and pasted from the clipboard as JSON. Loading the editor reads only the project's metadata. Property conditions, file pickers and unsupported kinds are not offered, and the still preview does not show the edits; the desktop does.

## Automatic playback rules

Settings → Automatic playback (or `apolline-wall policy`) decides what happens on lock, before sleep, on battery and under windows that hide the desktop: fullscreen ones on X11, maximized or fullscreen ones on Plasma through its wallpaper plugin. Lock and sleep pause by default; battery defaults to keep running. **Reduce** caps shaders and scenes at 15 fps and 50 % resolution without raising a lower limit of yours; videos keep their quality, so choose **Pause** to stop them instead. Rules belong to the daemon and survive a restart; conditions themselves don't.

- System state comes from logind (or elogind) and UPower over D-Bus signals, with no polling while connected. A service not on the bus is waited for through `NameOwnerChanged`; what it said last is cleared when it goes. Settings shows which integrations are available. No D-Bus service is needed for manual controls.
- A sleep delay inhibitor, when permitted, holds sleep until the pause is processed, for at most a second.
- Lock detection needs logind's LockedHint, or the shell's own `apolline-wall session locked|unlocked` (child mode: `{"session":{"locked":...}}`), sent at start and on each change. The two are separate reasons; an unlocked hint doesn't cancel a shell-reported lock.
- Manual pause per monitor, global manual pause and each automatic reason are independent; unplugging AC or unlocking never clears a manual pause, and an automatic cap never overwrites your limits.
- **Fullscreen on X11.** A monitor covered entirely by one EWMH fullscreen window pauses, driven by window-manager events with no periodic scan; leaving fullscreen, moving, minimizing or switching workspace reevaluates it. Desktop windows, the daemon's own and the external renderer are excluded; hidden, other-workspace and translucent windows don't count, and partial windows aren't combined. Turn it off with `apolline-wall policy --fullscreen false`. Native Wayland fullscreen detection isn't done; there the compositor's frame callbacks already stop drawing under a covering window.

## Limits and safeguards

- **Runtime directory.** Private (0700), checked for ownership and symlinks. Without `XDG_RUNTIME_DIR`, a private `apolline-wall-UID` directory under the temporary folder. A daemon left on the old `$XDG_RUNTIME_DIR/apolline-wall.sock` is still found after an upgrade, and a new one won't start next to it. Several graphical sessions of one user aren't kept apart yet.
- **IPC.** Commands are capped at 2 MiB and subscriptions at 64; a closed client releases its subscription at once. Decoding has a two-result queue, so a busy event loop pushes back instead of piling up full-sized images.
- **Child mode.** Events go out off the render loop through a short queue: a slow parent gets every event in order, and while the queue is full only the newest status waits. A broken output stream ends the child session; the parent restarts it and asks for the status again. A standalone daemon's stdout going away doesn't stop it.
- **Metadata.** `project.json` up to 8 MiB, regular files only; video and preview paths outside the project folder, absolute or through symlinks, are ignored. This is a path check, not a sandbox for the external renderer.
- **Thumbnails.** Two jobs at a time, shared by the library and the Workshop; inputs up to 32 MiB, 16384 pixels a side and a 128 MiB decode budget. GIFs sample the middle of their first 48 frames in two bounded passes. Video thumbnails come from ffmpeg with one thread, fitted within 640×400, and an eight-second deadline after which the process is killed and reaped. Files appear by atomic rename only when complete. The full picture preview has a 256 MiB limit.
- **Shaders.** Sources are regular UTF-8 files up to 1 MiB. Time accumulates in f64 on the CPU (the GPU still sees f32); gaps are capped at the larger of 250 ms and two frame intervals, so a long interruption doesn't jump; `iFrame` saturates instead of overflowing. Timers carry the wallpaper's generation and are cleared on pause, replacement and redraw, so a callback from a replaced shader can't drive its successor.
- **Posters.** Encoded on a worker to a temporary file, flushed and renamed; a failed write keeps the previous PNG and is a warning, not a failure. Readiness is tied to the monitor's current wallpaper, so a late poster never announces one the monitor has already left.

## Trying it

```sh
cargo build --release
cargo test --workspace
tools/headless.sh shots/   # under a throwaway headless sway, a screenshot per step
```

`tools/headless.sh` needs sway and grim. It shows the pictures in `tests/pictures`, crossfades, pauses, and checks that a missing file and an unknown command come back as errors. With ffmpeg and mpv it plays a generated clip, checks that it moves and that a paused one doesn't, and fades back to a picture. It runs `tests/shaders/hills.frag` the same way, checking that a new accent shows on a paused shader. With swaybg a stand-in plays linux-wallpaperengine's part: a scene shows, its process stops while paused and is gone after the next wallpaper, and one that crashes leaves the desktop as it was. With apolline-scene beside this repository it draws a synthetic scene in the daemon and checks that nothing went to the external renderer. steamcmd is checked against `tests/steamcmd/stand-in.sh`.

CI formats, lints with `clippy -D warnings` and tests on Ubuntu 22.04, x86_64 and arm64, and builds the `.deb`.

## What is stable

Run on real hardware (Arch, niri, an NVIDIA RTX 4070 Ti, two monitors, one turned): pictures, videos, shaders, native scenes and Apolline's integration. The app's playlist controls and tray are compiled and unit-tested, but their layout still needs a graphical check.

Written and tested only against stand-ins or synthetic input so far: GNOME through the extension, X11 sessions, real `steamcmd` downloads, linux-wallpaperengine as the fallback, real sleep, lock and battery transitions, X11 fullscreen pause, and Mesa (AMD and Intel) on real Workshop scenes. Audio for scenes is written but not yet run. KDE Plasma is written but not yet run. Not done: Fedora and Flatpak packages. Reports are welcome; [`docs/ROADMAP.md`](docs/ROADMAP.md) has the plan and [`docs/STATUS.md`](docs/STATUS.md) where things stand.

## License

MIT or Apache-2.0, at your option. Geist is under the SIL Open Font License, see `app/assets/fonts/OFL.txt`. Wallpaper Engine content stays its authors'; nothing of it is shipped or redistributed.
