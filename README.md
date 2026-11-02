# apolline-wall

Live wallpapers for Linux: pictures, videos, Shadertoy shaders and Wallpaper Engine's video wallpapers, on every monitor, crossfading between them and costing nothing while nothing shows. It draws on its own layer-shell surface per monitor with GLES, at each monitor's physical resolution, so it runs on niri, Hyprland, sway, river, labwc, Wayfire and COSMIC. It is also the wallpaper process of [Apolline](https://github.com/Pisosenie/apolline), which starts it and drives it, so video decoding never ends up inside the shell. Arch first; see [`docs/ROADMAP.md`](docs/ROADMAP.md) for the Workshop, scenes, KDE, GNOME and the rest.

## Using it

Install `apolline-wall-git` from the AUR (or `packaging/arch` here), stop whatever draws your wallpaper now (hyprpaper, swww, swaybg), and start the daemon with your session:

| Session | Line |
|---|---|
| niri | `spawn-at-startup "apolline-wall" "daemon"` in `config.kdl` |
| Hyprland | `exec-once = apolline-wall daemon` |
| sway | `exec apolline-wall daemon` |
| any session with systemd's `graphical-session.target` | `systemctl --user enable --now apolline-wall` |

Then pick wallpapers in the **Wallpapers** app (`apolline-wallpapers`), or from a terminal:

```sh
apolline-wall set ~/Pictures/dunes.jpg                  # every monitor
apolline-wall set ~/Videos/rain.mp4 --output DP-1       # one monitor
apolline-wall set ~/shaders/hills.frag --transition 0
apolline-wall limits --fps 20 --scale 0.5               # how hard shaders may work
apolline-wall pause                                      # and resume
apolline-wall status
```

`set` waits until the wallpaper shows and says why if it can't. The daemon remembers what each monitor shows and the limits in `~/.config/apolline-wall/state.toml`, so a login comes back to the same desktop. A wallpaper that fails is not written there; the last one that showed stays. Under Apolline the shell decides instead.

## The app

`apolline-wallpapers` is a window to browse and set wallpapers, in Apolline's look, on any desktop. It talks to the daemon over the same socket as the CLI, and starts the daemon if it isn't running.

- **Library.** Pictures, videos and shaders from `~/Pictures/Wallpapers`, `~/Videos/Wallpapers`, `~/.local/share/apolline-wall/wallpapers` and any folder you add in Settings.
- **Wallpaper Engine.** Everything Wallpaper Engine has on this machine: Workshop subscriptions and its own projects, from every Steam library, the native client and the Flatpak alike. Install Wallpaper Engine through Steam with Proton and subscribe in the Workshop as you would on Windows. Its video wallpapers play here. Scenes, web and application wallpapers are listed, with the reason they can't be set yet.
- **Steam Workshop.** Wallpaper Engine's Workshop, browsed without an account: videos, scenes or everything, trending, newest or most subscribed, and search. Items rated Questionable show only if you turn them on in Settings; Mature ones never do. **Subscribe in Steam** opens the item in the Steam client; once Steam has downloaded it, it appears in your library by itself and can be set from the same place. `apolline-wallpapers --page workshop` opens straight on this tab.
- **Inspector.** Pick a wallpaper, then every monitor or one, and whether it fills the screen or shows whole.
- **Now playing.** What each monitor shows, with pause.

Thumbnails are cached in `~/.cache/apolline-wall`; videos get theirs from ffmpeg, shaders once the daemon has shown them, and animated Workshop previews a frame from their middle. Workshop titles in Chinese, Japanese or Korean need a CJK font such as `noto-fonts-cjk`.

## Talking to it

Every instance listens on `$XDG_RUNTIME_DIR/apolline-wall.sock`, one JSON command per line in and one event per line out; a second instance finds the socket alive and leaves. Apolline starts it without arguments and talks over stdin and stdout instead, and then it exits when stdin closes, so it never outlives the shell.

| Command | Does |
|---|---|
| `{"show":{"source":"/abs/path.jpg"}}` | every monitor; also `"output":"DP-1"` for one, `"fit":"cover"\|"contain"`, `"transition_ms":400` |
| `{"pause":{"paused":true}}` | no animation, videos and shaders stop, changes land at once; `"output"` for one monitor |
| `{"limits":{"fps":30,"scale":0.5}}` | how hard shaders may work: frames per second (1–240, 30 to start with) and the share of the monitor's resolution they render at (0.1–1); either can be left out |
| `{"uniforms":{"accent":"#86bdf0"}}` | the shell's accent, which shaders read as `iAccent` |
| `{"status":{}}` | answered to whoever asked with `{"status":{"outputs":[{"name","source","paused"}],"fps","scale"}}` |

Events go to stdout and to every socket client. It answers `{"ready":{"output","source","poster"}}` once a monitor shows the new wallpaper, where `poster` is a still image of it, and `{"error":{"output","source","message"}}` when a file can't be read or a line isn't a command. A file that failed is not retried until the next `show`. Static pictures cost nothing between changes: no frames are drawn and no timers run.

## Videos

A `source` ending in `.mp4`, `.webm`, `.mkv`, `.mov`, `.gif` and the like plays muted and looping through libmpv, with hardware decoding where the driver offers it. libmpv is opened only when the first video is shown, so pictures work without mpv installed and a video then answers with an error saying so.

- One decode feeds every monitor showing the same file, at the size the most demanding monitor needs and never above the video's own.
- A frame is rendered only when a monitor showing the video is ready for one. When a fullscreen window covers the wallpaper the compositor stops asking, and decoding stops with it: in the test VM a 1080p clip went from about seven cores to under one, under both sway and niri.
- mpv runs in its simplest rendering mode: no HDR tone mapping or fancy scalers, which a wallpaper doesn't need and a laptop battery would pay for.
- The poster frame is written to `$XDG_RUNTIME_DIR/apolline/poster-*.png` before `ready` is sent, for the shell's accent and the lock screen.

## Shaders

A `source` ending in `.frag` or `.glsl` is a fragment shader written for [Shadertoy](https://www.shadertoy.com): a `mainImage(out vec4 fragColor, in vec2 fragCoord)` with `iTime`, `iTimeDelta`, `iFrame`, `iFrameRate`, `iResolution`, `iDate` and `iChannel0`–`iChannel3`, which all hold a tiling noise texture since there is nowhere to pick another yet. On top of those, `iAccent` is the shell's accent as RGB in 0–1, so a wallpaper can follow the mode you're in. Most single-pass shaders from Shadertoy paste in as they are; multi-pass buffers, sound and keyboard input are not supported. Mind their licenses: Shadertoy's default is CC BY-NC-SA. [`tests/shaders/hills.frag`](tests/shaders/hills.frag) is one of ours.

- A shader draws into a canvas of its own per monitor, about a 1080p screen's worth of pixels unless `limits` says otherwise, and the result is stretched over the monitor. It draws at most `fps` times a second and only when the compositor asks for a frame, so a covered or paused shader costs nothing and no timers keep running.
- A paused shader keeps its clock still and resumes where it stopped; a new accent shows at once, on paused shaders too.
- A shader that doesn't compile comes back as an `error` with the compiler's message, and its line numbers point into your file.
- The poster frame is its first frame, written as a PNG like a video's.

## Trying it

```sh
cargo build --release
tools/headless.sh shots/   # Linux: under a throwaway headless sway, a screenshot per step
```

`tools/headless.sh` needs sway and grim; it shows the pictures in `tests/pictures`, crossfades, pauses, and checks that a missing file and an unknown command come back as errors. With ffmpeg and mpv installed it also plays a generated clip, checks that it moves and that a paused one doesn't, and fades from it back to a picture. Then it runs `tests/shaders/hills.frag`, checks that it moves, that a paused one doesn't and that a new accent shows anyway, and fades back to a picture.

## Next

Live wallpapers for any Linux desktop, not only Apolline: a daemon and a CLI, then an app to pick and tune wallpapers, then KDE, GNOME and X11, wallpaper packages and the Steam Workshop. See [`docs/ROADMAP.md`](docs/ROADMAP.md).

## License

MIT or Apache-2.0, at your option.
