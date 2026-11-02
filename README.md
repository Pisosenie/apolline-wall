# apolline-wall

The wallpaper process of [Apolline](https://github.com/Pisosenie/apolline), a desktop shell for niri. It draws the wallpaper on its own layer-shell surface per monitor, with GLES, at each monitor's physical resolution, and crossfades between pictures. The shell starts it and tells it what to show; it is a program of its own so that video decoding never ends up inside the shell. It is not wired into the shell yet.

## Talking to it

It reads one JSON command per line on stdin, writes one event per line on stdout and exits when stdin closes, so it never outlives whoever started it.

| Command | Does |
|---|---|
| `{"show":{"source":"/abs/path.jpg"}}` | every monitor; also `"output":"DP-1"` for one, `"fit":"cover"\|"contain"`, `"transition_ms":400` |
| `{"pause":{"paused":true}}` | no animation, changes land at once; `"output"` for one monitor |

It answers `{"ready":{"output","source","poster"}}` once a monitor shows the new picture, where `poster` is a still image of it, and `{"error":{"output","source","message"}}` when a file can't be read or a line isn't a command. Static pictures cost nothing between changes: no frames are drawn and no timers run.

## Trying it

```sh
cargo build --release
tools/headless.sh shots/   # Linux: under a throwaway headless sway, a screenshot per step
```

`tools/headless.sh` needs sway and grim; it shows the pictures in `tests/pictures`, crossfades, pauses, and checks that a missing file and an unknown command come back as errors.

## Next

Video through libmpv's render API with hardware decoding and one decode for every monitor, then shaders with Shadertoy's uniforms plus the shell's accent, then wallpaper packages: a folder with a `wallpaper.toml` whose properties become controls in Apolline's settings. A hidden or covered wallpaper draws nothing, and a fullscreen game gets the poster frame instead.

## License

MIT or Apache-2.0, at your option.
