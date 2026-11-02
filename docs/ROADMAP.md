# apolline-wall roadmap

The goal: live wallpapers on Linux that feel as finished as [WallpaperMachine](https://github.com/MMFoundation/WallpaperMachine) does on macOS, with Wallpaper Engine's own wallpapers as the main source. An app to browse, pick and tune them; a daemon that keeps them running and costs nothing when unseen; any desktop. Apolline is one of the shells that drives it. Arch comes first, then every other distribution.

## Where it runs

| Desktop | How | When |
|---|---|---|
| niri, Hyprland, sway, river, labwc, Wayfire, COSMIC | layer-shell's background layer | now |
| Apolline | the shell starts it as a child and drives it over stdin | wired on Apolline's `wall` branch, parked |
| KDE Plasma | plasmashell paints its desktop above the background layer, so on KWin it takes the bottom layer instead, or a Plasma wallpaper plugin leaves the desktop transparent | step 3 |
| GNOME | no layer-shell: a regular window that a small Shell extension pins behind everything, the way Hanabi does | step 3 |
| X11 sessions | a desktop-type window per monitor | step 3 |

## Done

- Pictures, videos through libmpv, and Shadertoy shaders with the shell's accent. Each monitor gets its own wallpaper or follows the default, and changes crossfade.
- A covered or paused wallpaper costs nothing: drawing waits for frame callbacks, so decoding and shading stop when the compositor stops asking.
- Poster frames for the accent and the lock screen. An Arch package, `apolline-wall-git`.
- On its own: `apolline-wall daemon` for any session's autostart and a systemd user unit; one socket per session that the CLI, the app and Apolline share, and a second instance refuses to start; `set`, `pause`, `resume`, `limits`, `status`. What each monitor shows comes back after a login, and a wallpaper that fails gives way to the last one that showed.
- The app, `apolline-wallpapers`, first version:
  - A library of your folders plus Wallpaper Engine's: Workshop subscriptions, its default projects and yours, found in every Steam library (native and Flatpak). Posters, filters by kind, search.
  - An inspector with which monitor and how to fit.
  - A now-playing bar with pause, and settings for folders and shader limits.
  - Wallpaper Engine videos play. Scenes, web and application wallpapers are listed with the reason they can't be set yet.
  - If the daemon isn't running, the app says so and starts it.
- Looks: brightness, contrast, saturation and a mirror for anything, drawn in the final shader so they cost nothing, and speed for videos (mpv) and shaders (their clock). A live draft on the desktop with Apply and Revert, remembered per wallpaper and per monitor.
- A tray (StatusNotifierItem): pause, resume and random per monitor, and the window. It waits for the bar at login, and starts through XDG autostart from Settings. Every client gets a fresh status after a change, so the tray and the window agree.
- The Steam Workshop tab:
  - It browses without an account: Steam Community's browse page for the list, `GetPublishedFileDetails` for the details. Videos, scenes or everything; trending, newest or most subscribed; search.
  - Ratings: Questionable is off by default, Mature never shows.
  - Previews are cached, with a frame from the middle of animated ones.
  - **Subscribe in Steam** hands the item to the Steam client. The app watches for the download, only while one is pending, and the item shows up in the library ready to set.

## Next

### 1. The rest of the Workshop
- Collections and authors through `GetCollectionDetails`, and a size cap on the preview cache.
- For people without the Steam client: downloads through `steamcmd`.
  - It needs an account that owns Wallpaper Engine (app 431960), checked with `licenses_for_app 431960` first.
  - The password and the Steam Guard code go only to steamcmd's prompt and are never stored.
  - A download lands in staging and moves into place in one step.
  - A daily update check compares `time_updated`.
- The content stays its authors'. Nothing is redistributed.

### 2. Properties and the rest of the app
- A wallpaper's own properties as controls: Wallpaper Engine's `general.properties` and ours from `wallpaper.toml`, with named presets exported and imported.
- `fit: "blur"` fills mismatched edges with a blurred copy.
- Favorites and collections. Monitors picked on a drawing of their layout. Previous and next in the tray once playlists exist.
- "Not working? Report it" on a wallpaper, with a diagnostics file that has home paths and user names removed.

### 3. Every desktop
- KDE Plasma, GNOME and X11 as in the table above, each tested in a VM before it's called supported.
- Settings keyed by the monitor itself (make, model, serial), not by its connector. DP-1 can become DP-2 after a replug.

### 4. Rules
- One presentation policy: the strongest of sleep, lock, pause rules and other audio wins. Reduced quality caps fps and scale without touching the user's own pause.
- Battery (UPower): lower fps and scale, or show the poster; restore on AC. Sleep and lock (logind) pause everything, and every wallpaper is checked again after a wake.
- Playlists per monitor: a folder, favorites or a collection, on a timer or at login, with day and night variants by sunrise and sunset.
- A wallpaper that crashes the daemon twice in a row is skipped until it's set again.
- Under Apolline these come from its modes instead.

### 5. Scenes
- At first, hand those monitors to `linux-wallpaperengine` (optional dependency) running as its own process.
- Then a scene renderer of our own on GLES, translating Wallpaper Engine's shaders with naga, written clean-room.

### 6. Every distribution
- The AUR first. Then Fedora COPR, Debian and Ubuntu packages, and a Flatpak if the sandbox allows layer-shell.

### Later
- Shaders and scenes that react to audio: a PipeWire capture into an FFT texture.
- Web wallpapers.

## Borrowed from WallpaperMachine

Ideas only. Its code is GPL-3.0 and ours is MIT or Apache-2.0, so nothing is copied.
- Keyless Workshop browsing, and Steam credentials that are never stored.
- Downloads that stage and then swap in.
- Edits as a live draft with Apply and Revert, and presets.
- One presentation policy for pause and quality, with the user's pause kept apart.
- Settings that follow a monitor, not its connector.
- Skipping a wallpaper that keeps crashing.
- A report button next to every wallpaper.

## Principles

1. **Nothing to babysit.** One package, one autostart line, and the same desktop after every login.
2. **Free when unseen.** No timers or decoding while nothing is visible. Battery-aware by default.
3. **One protocol.** Apolline, the CLI and the app all speak the same JSON lines, so nothing has two code paths.
4. **Errors say what to do.** A file that can't be shown names itself and the reason, and the rest keeps working.
