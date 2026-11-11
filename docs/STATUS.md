# Where things stand

Handoff note, 2026-11-06, at the commit that pins apolline-scene `e96dadd`. Development moves
to the Linux machine. [ROADMAP.md](ROADMAP.md) lists what's done and what's planned. The
renderer's own state is in apolline-scene's
[docs/STATUS.md](https://github.com/Pisosenie/apolline-scene/blob/main/docs/STATUS.md).

## Scenes

The daemon prepares each scene with apolline-scene first (`src/native.rs`, on the loading
thread). If preparation fails, the scene goes to `linux-wallpaperengine`.
`APOLLINE_WALL_SCENES=external` sends every scene there. Of the 81 scenes in the local Workshop
set, 55 are drawn natively.

## On the Linux machine (2026-11-06)

Arch, niri, NVIDIA RTX 4070 Ti, two monitors. fmt, clippy and the 123 tests pass on x86_64.

- Native scenes run in the daemon on both monitors through niri's DRM backend, the turned one
  too. A particle-heavy scene costs about 31 % of a core on two screens at 30 fps; paused, nothing.
- Apolline runs it as its child now (Apolline `4ec9374`): a video, shader or project folder as
  `[desktop] wallpaper` starts it, a picture stops it, and its poster stands in when it can't run.
- Fixed: a Workshop "video" that was really a playlist or EDL had mpv fetch a URL (checked with
  a local HTTP server) or open other files. mpv now reads through ffmpeg only, local files only.
- Fixed: video posters failed on NVIDIA (`screenshot-to-file` under the render API) and the
  error path handed the video file itself out as the poster. They're read back from the canvas.
- `status` says `battery_available: true` on a desktop: it means UPower answers, not that there
  is a battery. Fine, if a confusing name.

## Never run on real hardware

All of these were only type-checked for Linux from the Mac (see below):

- KDE Plasma 6 (the bottom layer and the wallpaper plugin in `plasma/`, on the `kde-plasma`
  branch), GNOME through the extension in `gnome/`, and X11 sessions;
- real `steamcmd` downloads;
- `linux-wallpaperengine` as the fallback.

## First things to check on the machine

1. Build and install the package (`packaging/arch/PKGBUILD`, pinned to an apolline-scene
   commit), then run `apolline-wall daemon` in a niri or Hyprland session.
2. In apolline-scene, run `bash tools/gles-check.sh` and
   `tools/render-all.sh ~/.local/share/Steam/steamapps/workshop/content/431960 <assets>`,
   using the Wallpaper Engine `assets` folder from the Steam library. Mac desktop GL forgives
   int↔float mixes that GLES doesn't, so scenes may fail here that drew on the Mac.
3. Check whether a scene that prepares but fails to compile its shaders at `Renderer::new`
   falls back to `linux-wallpaperengine`, or just shows the error. Preparation failures do fall
   back; renderer construction failures weren't tried.
4. Watch the frame rate and the GPU budget on 4K scenes. The budget is 1 GiB of textures and
   targets per scene.

## Not done

- **Audio, unchecked.** Written on the `fix/review-findings` branch, never run:
  - While an unpaused monitor shows a scene that listens, `src/engine/audio.rs` runs `parec` on
    the default output's monitor.
  - `src/spectrum.rs` turns what it hears into 64 bands a side.
  - Those bands go to the renderer each frame.
  - The banding and loudness scale are guesses: 30 Hz to 16 kHz, a 60 dB range. Compare them
    with Wallpaper Engine on Windows if a scene looks too quiet or too busy.
- KDE Plasma: dmabuf or an asynchronous readback instead of reading each frame back in place,
  once the Plasma check gives numbers (step 3); the rest is in ROADMAP.md.

## Checking Linux code from a Mac

`src/engine*` is Linux-only, so a check on the Mac skips it. Type-check it with
`cargo clippy --target x86_64-unknown-linux-gnu --all-targets`. Set
`CC_x86_64_unknown_linux_gnu` and `AR_x86_64_unknown_linux_gnu` to stub scripts: the C compiler
stub only creates its `-o` output, and the archiver stub writes `!<arch>\n`. On the Linux machine
none of this is needed.

The `pipewire` crate also needs PipeWire's headers for bindgen. Take `src/pipewire` and
`spa/include` from a PipeWire release tarball, write `pipewire/version.h` from `version.h.in`,
and use Alpine's `musl-dev` and `linux-headers` packages for libc headers. Then set:
- `PKG_CONFIG` to a script that prints `-I` flags for `libpipewire-0.3` and `libspa-0.2`;
- `PKG_CONFIG_ALLOW_CROSS=1`;
- `LIBCLANG_PATH` to the Command Line Tools' `usr/lib`;
- `BINDGEN_EXTRA_CLANG_ARGS_x86_64_unknown_linux_gnu` to
  `--target=x86_64-unknown-linux-musl -nostdinc`, with clang's own include folder and the musl
  headers as `-isystem`.
