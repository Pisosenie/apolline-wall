# Changelog

## 0.2.0 — 2026-11-12

- **A folder as a playlist**: `apolline-wall playlist FOLDER --every SECONDS` plays what a folder holds, read again before every step, so what's added joins and what's gone is skipped; the app offers to rotate a library folder from its inspector.
- **Scenes hear the music**: audio-reactive Wallpaper Engine scenes get what's playing, through `parec`, in 64 bands a side.
- **KDE Plasma 6**: a wallpaper plugin plays each monitor from a PipeWire stream under Plasma's icons and widgets, and monitors without it get the wallpaper on the bottom layer. The Debian package for Ubuntu 22.04 leaves the plugin out: its PipeWire is too old.
- A paused monitor stops drawing a scene the others still show; a scene's canvas is only the part its monitors show, so a portrait monitor stays sharp; canvases shrink when a monitor leaves.
- Project folders are read again only when they change; a poster that can't be saved falls back to the video file; steamcmd signs in from a private script, so the account name isn't on its command line.
- Built against apolline-scene 0.2.0: bigger scenes load, timelines on particle overrides play.

## 0.1.0 — 2026-11-07

The first release: live wallpapers for Linux, and the wallpaper process of Apolline.

- **Pictures, videos and Shadertoy shaders** on every monitor, each with its own wallpaper, fit (cover, contain, or blurred edges), look and playlist, crossfaded, on Wayland compositors with layer-shell and, untested yet, GNOME and X11.
- **Wallpaper Engine** videos and scenes from your Workshop subscriptions, in every Steam library. Scenes are drawn natively by apolline-scene; what it can't draw yet, and web wallpapers, go to linux-wallpaperengine when it is installed.
- **One run for many monitors**: a video decodes once and a scene runs once into a canvas of its own shape, whatever the number of monitors showing it.
- **Nothing spent while nothing shows**: paused while locked, asleep or (optionally) on battery, and by the shell in game modes.
- **apolline-wallpapers**, an app and tray to browse your library and the Workshop, set wallpapers per monitor, adjust their look and properties, and report a problem.
- **Safe with anyone's files**: Workshop items are read with size limits and inside their folder, videos play local files only, scene scripts run sandboxed.
- A CLI, a JSON-lines socket, and child mode for Apolline over stdin and stdout.
