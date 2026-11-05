# apolline-wall roadmap

The goal: live wallpapers on Linux that feel as finished as [WallpaperMachine](https://github.com/MMFoundation/WallpaperMachine) does on macOS, with Wallpaper Engine's own wallpapers as the main source. An app to browse, pick and tune them; a daemon that keeps them running and costs nothing when unseen; any desktop. Apolline is one of the shells that drives it. Arch comes first, then every other distribution.

## Where it runs

| Desktop | How | When |
|---|---|---|
| niri, Hyprland, sway, river, labwc, Wayfire, COSMIC | layer-shell's background layer | now |
| Apolline | the shell starts it as a child and drives it over stdin | wired on Apolline's `wall` branch, parked |
| KDE Plasma | plasmashell paints its desktop above the background layer, so on KWin it takes the bottom layer instead, or a Plasma wallpaper plugin leaves the desktop transparent | step 3 |
| GNOME | no layer-shell: a window per monitor that apolline-wall's Shell extension (`gnome/`) puts behind every window, on every workspace, out of the overview and Alt+Tab; without the extension, an X11 desktop window through Xwayland | written, not yet run on GNOME |
| X11 sessions (Xfce, Cinnamon, MATE, i3, Openbox…) | a `_NET_WM_WINDOW_TYPE_DESKTOP` window per RandR monitor, below, sticky, out of taskbars and pagers | written, not yet run on X11 |

## Done

- Pictures, videos through libmpv, and Shadertoy shaders with the shell's accent. Each monitor gets its own wallpaper or follows the default, and changes crossfade.
- Paused native playback stops animation work. Wayland drawing follows frame callbacks; X11 additionally pauses fully covered monitors using EWMH fullscreen events. Actual compositor behavior still needs verification.
- Poster frames for the accent and the lock screen. An Arch package, `apolline-wall-git`.
- On its own: `apolline-wall daemon` for any session's autostart and a systemd user unit; one socket in the user runtime directory that the CLI, the app and Apolline share (simultaneous graphical sessions are not isolated yet), and a second instance refuses to start; `set`, `pause`, `resume`, `limits`, `status`. What each monitor shows comes back after a login, and a wallpaper that fails gives way to the last one that showed.
- The app, `apolline-wallpapers`, first version:
  - A library of your folders plus Wallpaper Engine's: Workshop subscriptions, its default projects and yours, found in every Steam library (native and Flatpak). Posters, filters by kind, search.
  - An inspector with which monitor and how to fit.
  - A now-playing bar with pause, and settings for folders and shader limits.
  - Wallpaper Engine videos play here. Application wallpapers are listed with the reason they can't be set.
  - If the daemon isn't running, the app says so and starts it.
- Looks: brightness, contrast, saturation and a mirror for everything we draw ourselves, drawn in the final shader so they cost nothing, and speed for videos (mpv) and shaders (their clock). A live draft on the desktop with Apply and Revert, remembered per wallpaper and per monitor.
- A tray (StatusNotifierItem): pause, resume and random per monitor, and the window. It waits for the bar at login, and starts through XDG autostart from Settings. Every client gets a fresh status after a change, so the tray and the window agree.
- Scenes and web wallpapers through `linux-wallpaperengine` (optional dependency), one process per monitor while our surface steps aside. Pause stops the process, it dies with the daemon, and one that crashes gives its last words in the error and puts the previous wallpaper back. Without it the app says what to install. `tools/headless.sh` checks all of it with a stand-in.
- The Steam Workshop tab:
  - It browses without an account: Steam Community's browse page for the list, `GetPublishedFileDetails` for the details. Videos, scenes or everything; trending, newest or most subscribed; search.
  - Ratings: Questionable is off by default, Mature never shows.
  - Previews are cached, with a frame from the middle of animated ones.
  - **Subscribe in Steam** hands the item to the Steam client. The app watches for the download, only while one is pending, and the item shows up in the library ready to set.
  - Collections through `GetCollectionDetails`, and an author's work from their Workshop page. Collections carry no rating, so each is judged by its first four wallpapers.
  - Without the Steam client, downloads through `steamcmd`, driven in a pseudo-terminal of its own: the password and the Steam Guard code go only to its prompt, and the app keeps only the account name. Signing in checks `licenses_for_app 431960`. steamcmd downloads into its own staging folder and moves a finished item into place. Updates are found by comparing `time_updated` with what steamcmd recorded, each time the tab opens. Tested against a stand-in, `tests/steamcmd/stand-in.sh`.
  - The preview cache stays under 150 MB.

## Next

### 1. The rest of the Workshop
- Wallpaper Engine's own assets through steamcmd (`app_update 431960` for the Windows platform), so linux-wallpaperengine draws scenes without the Steam client.
- Check steamcmd against a real account on x86_64 Arch: so far only the stand-in has answered.
- The content stays its authors'. Nothing is redistributed.

### 2. Properties and the rest of the app
- Wallpaper Engine `general.properties` controls are implemented for bool, slider, color, combo and text. Still needed: named property presets with export/import, `wallpaper.toml`, scripts/conditions and animated preview of property edits.
- `fit: "blur"` fills mismatched edges with a blurred copy.
- Favorites and ordered per-monitor/shared playlists are implemented. Still needed: user collections and monitor layout selection. Previous/next/stop and live playlist summaries are now available in the tray and playback cards; graphical verification remains pending.
- "Not working? Report it" on a wallpaper, with a diagnostics file that has home paths and user names removed.

### 3. Every desktop
- GNOME is written: without layer-shell the daemon opens an xdg window per monitor, titled `apolline-wall:<connector>`, but only if its Shell extension is enabled, so it never covers the desktop. It needs a first run on GNOME 45–49.
- X11 is written. The daemon is now an engine (pictures, videos, shaders, fades, the protocol) and two drivers that own the surfaces and frame pacing: Wayland (layer-shell, or GNOME's windows) and X11 (a desktop window per RandR monitor, EGL through Xlib, a 16 ms timer where Wayland has frame callbacks, libmpv told the X display). Wayland comes first; a Wayland session that can't host a background falls back to Xwayland. It needs a first run on an X11 session, and on desktops that draw their own icons window (xfdesktop, Nemo, Caja) whether ours stays under it.
- X11 fullscreen pause is implemented through EWMH events, independently per monitor, with an opt-out. Real WM verification is still needed; arbitrary occlusion and Wayland fullscreen detection are not covered.
- KDE Plasma: KWin has layer-shell; whether plasmashell's desktop stays above our background surface needs a run on Plasma 6, and a Plasma wallpaper plugin is the fallback.
- Each tested on its desktop before it's called supported.
- Settings follow the monitor itself: the daemon remembers which monitor each connector had, by what the compositor calls it on Wayland (its description, usually with a serial) and by its EDID on X11 (maker, name, serial), and when that monitor comes back on another connector that has nothing of its own, its wallpaper, look and playlist move with it. Two monitors that look the same don't move.

### 4. Rules
- Implemented: independent manual, logind/shell lock, sleep, battery and X11 fullscreen pause reasons. Battery reduction caps shader FPS and resolution without raising or overwriting user limits. Audio rules are still pending.
- UPower and logind follow D-Bus signals. A service that isn't on the bus is waited for through `NameOwnerChanged`, with no polling, and what it said last is cleared when it goes. Lock requires LockedHint reporting or the shell hook. A sleep delay inhibitor bounds pause acknowledgement. Real battery, sleep and lock transitions still need verification; there is no automatic poster-only mode yet.
- Ordered playlists run per monitor or on a shared clock, persist across restart and suspend while a participating monitor is paused. Folder feeds, collections, login rules and sunrise/sunset variants remain pending.
- Reported wallpaper load/playback failures are skipped for the lifetime of the running playlist; exhausted lists stop without retry timers, and manual selection permits retry. Detecting and persisting repeated crashes of the daemon itself remains pending.
- Apolline can supply lock state through the session command. Its integration is maintained separately and still needs wiring and verification.

### 5. Scenes of our own
- The renderer is its own library, [apolline-scene](https://github.com/Pisosenie/apolline-scene), written clean-room on GLES; its plan is in its `docs/PLAN.md`. The daemon links it and draws scenes in its own context, so scenes get looks, crossfades and one process for everything.
- Supported 2D scenes now render in the daemon through `apolline-scene`, with cover/contain, looks and crossfades. Static materials are flattened once with no animation timer. Active g_Time/g_Frametime fragment materials reuse the shader clock, FPS/quality limits and pause scheduler; Linux GLES runtime validation remains pending. Unsupported scenes still use `linux-wallpaperengine`, and a new `fps` limit reaches that scene only when it starts again; looks and speed don't apply to it.

### 6. Every distribution
- The AUR first. Then Fedora COPR, Debian and Ubuntu packages, and a Flatpak if the sandbox allows layer-shell.

### Later
- Shaders and scenes that react to audio: a PipeWire capture into an FFT texture.

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
