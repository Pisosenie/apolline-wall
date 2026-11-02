# apolline-wall

The wallpaper process of [Apolline](https://github.com/Pisosenie/apolline), a desktop shell for niri. It draws the wallpaper on its own layer-shell surface per monitor, with GLES, at each monitor's physical resolution, and crossfades between pictures and videos. The shell starts it and tells it what to show; it is a program of its own so that video decoding never ends up inside the shell. It is not wired into the shell yet.

## Talking to it

It reads one JSON command per line on stdin, writes one event per line on stdout and exits when stdin closes, so it never outlives whoever started it.

| Command | Does |
|---|---|
| `{"show":{"source":"/abs/path.jpg"}}` | every monitor; also `"output":"DP-1"` for one, `"fit":"cover"\|"contain"`, `"transition_ms":400` |
| `{"pause":{"paused":true}}` | no animation, videos stop, changes land at once; `"output"` for one monitor |

It answers `{"ready":{"output","source","poster"}}` once a monitor shows the new wallpaper, where `poster` is a still image of it, and `{"error":{"output","source","message"}}` when a file can't be read or a line isn't a command. A file that failed is not retried until the next `show`. Static pictures cost nothing between changes: no frames are drawn and no timers run.

## Videos

A `source` ending in `.mp4`, `.webm`, `.mkv`, `.mov`, `.gif` and the like plays muted and looping through libmpv, with hardware decoding where the driver offers it. libmpv is opened only when the first video is shown, so pictures work without mpv installed and a video then answers with an error saying so.

- One decode feeds every monitor showing the same file, at the size the most demanding monitor needs and never above the video's own.
- A frame is rendered only when a monitor showing the video is ready for one. When a fullscreen window covers the wallpaper the compositor stops asking, and decoding stops with it: in the test VM a 1080p clip went from about seven cores to under one, under both sway and niri.
- mpv runs in its simplest rendering mode: no HDR tone mapping or fancy scalers, which a wallpaper doesn't need and a laptop battery would pay for.
- The poster frame is written to `$XDG_RUNTIME_DIR/apolline/poster-*.png` before `ready` is sent, for the shell's accent and the lock screen.

## Trying it

```sh
cargo build --release
tools/headless.sh shots/   # Linux: under a throwaway headless sway, a screenshot per step
```

`tools/headless.sh` needs sway and grim; it shows the pictures in `tests/pictures`, crossfades, pauses, and checks that a missing file and an unknown command come back as errors. With ffmpeg and mpv installed it also plays a generated clip, checks that it moves and that a paused one doesn't, and fades from it back to a picture.

## Next

Shaders with Shadertoy's uniforms plus the shell's accent, then wallpaper packages: a folder with a `wallpaper.toml` whose properties become controls in Apolline's settings. A fullscreen game gets the poster frame instead of a running video.

## License

MIT or Apache-2.0, at your option.
