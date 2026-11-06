# Changelog

## 0.1.0 — 2026-11-07

The first release: live wallpapers for Linux, and the wallpaper process of Apolline.

- **Pictures, videos and Shadertoy shaders** on every monitor, each with its own wallpaper, fit (cover, contain, or blurred edges), look and playlist, crossfaded, on Wayland compositors with layer-shell and, untested yet, GNOME and X11.
- **Wallpaper Engine** videos and scenes from your Workshop subscriptions, in every Steam library. Scenes are drawn natively by apolline-scene; what it can't draw yet, and web wallpapers, go to linux-wallpaperengine when it is installed.
- **One run for many monitors**: a video decodes once and a scene runs once into a canvas of its own shape, whatever the number of monitors showing it.
- **Nothing spent while nothing shows**: paused while locked, asleep or (optionally) on battery, and by the shell in game modes.
- **apolline-wallpapers**, an app and tray to browse your library and the Workshop, set wallpapers per monitor, adjust their look and properties, and report a problem.
- **Safe with anyone's files**: Workshop items are read with size limits and inside their folder, videos play local files only, scene scripts run sandboxed.
- A CLI, a JSON-lines socket, and child mode for Apolline over stdin and stdout.
