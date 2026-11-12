# Working on apolline-wall

apolline-wall is live wallpapers for any Linux desktop: a daemon that draws them, a CLI, and the **Wallpapers** app to pick and tune them, with Wallpaper Engine's wallpapers and Steam Workshop as the main source. Arch comes first, then every other distribution. Apolline (Danil's shell, `~/src/apolline`) is one host that starts it as a child. Read `README.md` for what it does and `docs/ROADMAP.md` for the plan. The scene renderer lives in its own repository, `~/src/apolline-scene`, with its own `AGENTS.md`.

The idea comes from [WallpaperMachine](https://github.com/MMFoundation/WallpaperMachine) on macOS. It is GPL-3.0 and this is MIT or Apache-2.0, so take ideas from it, never code. The same goes for linux-wallpaperengine (GPL-3.0): it's only run as a separate program.

## The workspace

- `protocol/` (`apolline-wall-protocol`): the JSON lines everyone speaks (`protocol.rs`: commands, events, `Look`, the socket path, broadcasting), what a path is (`source.rs`: picture, video, shader, or a folder meaning a Wallpaper Engine scene), and what to install on which distribution (`install.rs`, by `/etc/os-release`).
- `src/` (`apolline-wall`, the daemon and CLI):
  - `main.rs`: no arguments is the child mode Apolline uses (commands on stdin), `daemon` is the standalone daemon, anything else is the CLI (`cli.rs`). `state.rs` keeps `~/.config/apolline-wall/state.toml`; a wallpaper that fails falls back to the last good one.
  - `engine.rs`: the engine. Commands, screens, decoding, crossfades, looks, pause. Screens are known by an id, never by a Wayland output. It doesn't know which display it's on except through two enums, `Desktop` and `Surface`.
  - `engine/wayland.rs` and `engine/wayland/shell.rs`: the Wayland driver. wlr-layer-shell background surfaces (the bottom layer under KDE, where Plasma's own desktop sits on the background one); on GNOME, which has none, one xdg window per monitor (`app_id` `apolline-wall`, title `apolline-wall:<connector>`) that the Shell extension in `gnome/` puts behind everything, opened only when that extension is enabled. Frame callbacks pace drawing, and a frame request has to go out before the commit it belongs to.
  - `engine/rotation.rs`: playlists, per monitor or shared. A folder playlist (`Playlist.folder`) is read with `playlist::folder_sources` when it starts and again before every step.
  - `engine/feed.rs`: KDE Plasma's way in. The wallpaper plugin in `plasma/` runs `apolline-wall feed <connector>`; that monitor's surface is hidden, its frames are drawn into an offscreen `gl::Target`, read back and sent to a PipeWire output stream (pipewire-rs, its loop's fd in calloop), whose node `status` reports. A 16 ms timer paces it while someone watches, and the plugin's `--covered` (Plasma's own maximized-window monitor) pauses it through the same `covered` flag X11 fullscreen uses. A fed monitor stays fed until the daemon restarts (under Plasma its own surface would cover the desktop); only a stream that breaks gives the surface back, until the plugin asks again. A new size renegotiates the same stream, since a new one could get the old node id and the plugin wouldn't notice.
  - `engine/x11.rs`: the X11 driver. A `_NET_WM_WINDOW_TYPE_DESKTOP` window per RandR monitor, Xlib opened with `x11-dl` for EGL and its XCB connection used through x11rb, a 16 ms timer instead of frame callbacks. If Wayland can't host a background (GNOME without the extension) and `DISPLAY` is set, the daemon uses this through Xwayland.
  - `engine/playback.rs` (videos through libmpv, one decode per file for every monitor), `engine/shading.rs` (Shadertoy shaders, fps limit, `iAccent`), `engine/scenery.rs` (Wallpaper Engine scenes handed to `linux-wallpaperengine` as a process per monitor: SIGSTOP while paused, fallback if it dies), `engine/socket.rs` (`$XDG_RUNTIME_DIR/apolline-wall/control.sock`; clients and the single-instance check still look at the old `apolline-wall.sock`).
  - `gl.rs` and `gl/shader.rs`: GLES 3 through glutin's EGL and glow. `video.rs`: our own FFI to libmpv, loaded with libloading only when the first video shows. `picture.rs`: decoding and fitting stills.
- `app/` (`apolline-wallpapers`, iced 0.14): `main.rs` (state and messages), `view.rs` and `view/{shop,look,steam}.rs`, `library.rs` (folders plus Wallpaper Engine's), `engine.rs` (Steam libraries and `project.json`), `workshop.rs` and `workshop/api.rs` (Workshop browsing without a key: browse pages, collections, authors, `GetPublishedFileDetails`, `GetCollectionDetails`), `browse.rs`, `steamcmd.rs` and `download.rs` (downloads without the Steam client, see below), `tray.rs` (StatusNotifierItem through ksni, Linux only), `config.rs` (`~/.config/apolline-wall/app.toml`), `daemon.rs` (talking to the daemon), `style.rs`.
- `gnome/apolline-wall@pisosenie.github.io/`: the GNOME Shell extension, GNOME 45–49, ES modules.
- `plasma/io.github.pisosenie.apolline-wall/`: the Plasma 6 wallpaper plugin, QML only: KPipeWire's `PipeWireSourceItem` plays the stream, Plasma5Support's executable data source runs `apolline-wall feed`.
- `packaging/`: `arch/PKGBUILD` (`apolline-wall-git`), the systemd user unit, the desktop file.
- `tests/`: pictures and a shader for `tools/headless.sh`, and stand-ins for programs we drive: `tests/scenes/stand-in.sh` for linux-wallpaperengine and `tests/steamcmd/stand-in.sh` for steamcmd (a made-up account `stand_in`, password `stand-in-password`, Steam Guard code `QX7RM`).

## steamcmd and Steam accounts

steamcmd runs in a pseudo-terminal of its own without echo. Its password and Steam Guard prompts are shown in the app, and the answers go only to that terminal: never on its command line, into a file, a log or `app.toml`, which keeps only the account name. Account names are checked (`valid_account`) so one can't be read as a steamcmd command. Its install folder is `~/.local/share/apolline-wall/steamcmd`, and the library reads its `steamapps/workshop/content/431960`. Never sign in with a real account yourself; test against the stand-in.

## Before saying something is done

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The daemon only builds on Linux (on macOS `serve` says so), so its code is checked on Linux. The machine for that is the OrbStack VM `apolline-dev` (Arch Linux ARM): `CARGO_TARGET_DIR=$HOME/target-wall`, test scripts in `~/wall-test`. Right now Danil wants only `cargo check`, clippy and tests there, with short output: no runs, screenshots or clicking, which cost too much. Another session runs Apolline's own sway and niri in that VM: never stop processes by name (`pkill -x sway`); stop only the pids you started.

`tools/headless.sh OUT` (Linux, sway and grim) drives the daemon under a headless sway and screenshots pictures, crossfades, pause, a video, a shader and scenes through the stand-in. Build with `cargo build --release --workspace` first: clippy and the script don't rebuild the app.

## How the code is written

- Comments in English, few, saying why. Doc comments are plain sentences. Danil writes in Russian; answer him in Russian.
- Errors say what to do ("install libmpv2 (`sudo apt install libmpv2`)"), name the file, and the rest keeps working. Messages that name a package go through `protocol::install::hint`.
- Nothing costs anything while unseen: no timers or decoding without a frame wanted.
- One protocol: the app, the CLI, the tray and Apolline all speak the same JSON lines.
- Files stay under about 800 lines; split by topic when they grow.

## Git

- Commit and push only when Danil says so ("коммить", "пушь").
- The message is one plain sentence ("Collections, authors, and Workshop downloads through steamcmd"), no `feat:` prefixes, no co-author lines.
- The author is `Danil Zorin <45851636+Pisosenie@users.noreply.github.com>`. Push to `git@github.com:Pisosenie/apolline-wall.git`.
- Never commit `.idea/` or `target/`.

## How Danil works

These are the rules the project was built under; they win over any generic ones.

- Work as a senior engineer: understand the whole thing first, then decide and recommend; don't hand him a menu or ask him the same thing twice. Within a planned stage, go from step to step without asking "take step N?"; stop only for a real product decision or a blocker.
- Code clean, light and maintainable: the simplest structure that reads well, nothing clever for its own sake. Match the code around it.
- Check your own changes before calling them done, by name: a value threaded into one branch but not its siblings, an extraction that left the original behind (grep for one definition), a doc comment claiming more than the code does, a default that takes too much.
- Estimates in the pace where the agent writes the code and Danil reviews: hours to days, not weeks per feature.
- Parallel agents cost his limit: count them before starting any, keep it small, and prefer slower and sequential to dozens at once.
- He pushes to GitHub as Pisosenie with the noreply address above, never anything else.

`~/.claude/rules/*.md` on his machine (from everything-claude-code) says TDD with 80% coverage, E2E with Playwright, conventional `feat:` commits and parallel agents always. For this project the rules above win: tests that check behaviour, plain commit sentences, few agents. Nothing in `~/.claude` skills was used to build it.

## Tools that were used

- The OrbStack VM `apolline-dev` (Arch Linux ARM, aarch64), with sway, niri, grim, wtype, mpv, ffmpeg, swaybg, wqy-zenhei, namcap, dbus. Headless sway: `WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway -c <conf>` with `output HEADLESS-1 resolution 1440x900`, `LIBGL_ALWAYS_SOFTWARE=1`, screenshots with `grim`. Nested niri runs inside it with the winit backend.
- Clicking the app there: headless sway's seat has no pointer, so `swaymsg seat … cursor press` reaches nothing. `tools/vpointer` (not in the workspace; `cargo build --release --manifest-path tools/vpointer/Cargo.toml`) moves and clicks a wlr virtual pointer: `vpointer X Y click`, `vpointer X Y scroll N`. Typing: `wtype`, which can drop the first key of each call, so send `wtype -k End text`.
- The tray: `dbus-run-session`, then `busctl` against the StatusNotifierItem's `com.canonical.dbusmenu` (layout depth 10, not -1).
- Packaging checks: `makepkg` in a temporary clone; namcap on Arch Linux ARM misreports which package owns a library, so check with `ldd` and `pacman -Qo`.

## Lessons from building it

- `cargo clippy` and `tools/headless.sh` build only what they need; after changing the app, `cargo build --release --workspace` before looking at it.
- The OrbStack VM has once read a half-written file from the shared folder: a strange parse error right after an edit goes away after a second's wait.
- In shell scripts under `set -o pipefail`, `grep -q` on a pipe fails the check; use `grep … > /dev/null`.
- niri doesn't map a layer surface again after it was unmapped with a null buffer, so leaving a scene opens a fresh surface; sway did either.
- mpv in the VM used about eight cores for a 1080p clip until `gpu-dumb-mode=yes`; `profile=fast` didn't help. Memory came down with one decoder thread, `malloc_trim` and `M_ARENA_MAX=2`.
- `sctk` with its default features pulls in libxkbcommon, which a wallpaper doesn't need; build paths leak into the binary without `--remap-path-prefix`; `gcc-libs` is a metapackage now, depend on `libgcc`.
- ksni refuses to start without a tray host unless `assume_sni_available(true)`: at login the tray often starts before the bar.
- Steam's Workshop pages are read only by their `filedetails/?id=` links, so markup changes don't break them; collections carry no rating at all.

## Where things stand

`docs/ROADMAP.md` is the current state: what is done, what still needs a real run, and what is left. Keep it true when the code changes, as short lists rather than a log.

Never run for real yet, only built and tested against stand-ins: GNOME, KDE Plasma and X11 sessions; real steamcmd and real linux-wallpaperengine (both need an x86_64 machine with Steam); D-Bus power rules against real UPower and logind; native scenes on GLES; speed, hardware decoding and battery use on real hardware.
