# apolline-wall roadmap

The goal: live wallpapers on Linux that feel as finished as [WallpaperMachine](https://github.com/MMFoundation/WallpaperMachine) does on macOS. An app to browse, pick and tune them; a daemon that keeps them running and costs nothing when unseen; any desktop. Apolline is one of the shells that drives it. Arch comes first, then every other distribution.

## Where it runs

| Desktop | How | When |
|---|---|---|
| niri, Hyprland, sway, river, labwc, Wayfire, COSMIC | layer-shell's background layer | now |
| Apolline | the shell starts it as a child and drives it over stdin | now, wiring in progress |
| KDE Plasma | plasmashell paints its desktop above the background layer, so on KWin it takes the bottom layer instead, or a Plasma wallpaper plugin leaves the desktop transparent | step 3 |
| GNOME | no layer-shell: a regular window that a small Shell extension pins behind everything, the way Hanabi does | step 3 |
| X11 sessions | a desktop-type window per monitor | step 3 |

## Done

- Pictures, videos through libmpv, and Shadertoy shaders with the shell's accent. Each monitor gets its own wallpaper or follows the default, and changes crossfade.
- A covered or paused wallpaper costs nothing: drawing waits for frame callbacks, so decoding and shading stop when the compositor stops asking.
- Poster frames for the accent and the lock screen. An Arch package, `apolline-wall-git`.
- On its own: `apolline-wall daemon` for any session's autostart and a systemd user unit; one socket per session that the CLI, the app and Apolline share, and a second instance refuses to start; `set`, `pause`, `resume`, `limits`, `status`. What each monitor shows comes back after a login, and a wallpaper that fails gives way to the last one that showed.

## Next

### 1. The app
- One window in Apolline's design language that runs on any desktop:
  - Library: a grid of posters from your folders, imports and the bundled set, with search and kinds (picture, video, shader).
  - Inspector: a live preview, properties as controls with presets, and apply to every monitor or one, picked on a drawing of your monitor layout.
  - A now-playing bar per monitor.
- A tray icon (StatusNotifierItem): pause, next and mute per monitor, open the app.
- Settings: library folders, performance (fps, scale, what happens on battery), launch at login.
- The app is only a client of the socket, like the CLI and Apolline.

### 2. Wallpapers as packages
- A folder with `wallpaper.toml`: name, kind, preview, and properties (color, number, toggle, choice) with defaults.
  - Shader properties arrive as uniforms.
  - Video properties map to mpv: speed, brightness, saturation, flip.
- Import from Wallpaper Engine: video items, which are `project.json` plus a video, become packages with their properties. Scene and web items are listed as not supported yet, with the reason.
- `fit: "blur"`: a picture or video that doesn't match the screen gets a blurred copy of itself behind it instead of black bars.

### 3. Every desktop
- KDE Plasma, GNOME and X11 as in the table above, each tested in a VM before it's called supported.

### 4. Rules
- Battery (UPower): lower fps and scale, or show the poster. Restore on AC.
- Sleep and lock (logind): pause everything.
- Playlists per monitor: a folder or favorites, on a timer or at login, with day and night variants by sunrise and sunset.
- Under Apolline these come from its modes instead.

### 5. Steam Workshop and scenes
- Browse the Workshop, and download through steamcmd with the user's account that owns Wallpaper Engine (app 431960). The content stays the authors'; nothing is redistributed.
- Scenes: hand those monitors to `linux-wallpaperengine` (optional dependency) running as its own process until a native scene renderer exists.

### 6. Every distribution
- The AUR first. Then Fedora COPR, Debian and Ubuntu packages, and a Flatpak if the sandbox allows layer-shell.

### Later
- Shaders that react to audio: a PipeWire capture into an FFT texture, the way Shadertoy does it.
- Web wallpapers.

## Principles

1. **Nothing to babysit.** One package, one autostart line, and the same desktop after every login.
2. **Free when unseen.** No timers or decoding while nothing is visible. Battery-aware by default.
3. **One protocol.** Apolline, the CLI and the app all speak the same JSON lines, so nothing has two code paths.
4. **Errors say what to do.** A file that can't be shown names itself and the reason, and the rest keeps working.
