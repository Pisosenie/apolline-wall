# Testing the review fixes

Branch `fix/review-findings`, on top of v0.1.0 (`b9730e5`): the review fixes (`4e6cc5d`), then
audio for scenes. Branch `kde-plasma` adds KDE Plasma on top of it (items 9 and 10), and `playlist-folders` a
folder as a playlist on top of that (item 11). They stay off
`main` until it has run on the Linux machine. It builds with apolline-scene's
`fix/review-findings`, whose own
[docs/TESTING.md](https://github.com/Pisosenie/apolline-scene/blob/fix/review-findings/docs/TESTING.md)
covers the renderer.

Checked so far:
- `cargo test --workspace` passes on the Mac.
- `cargo clippy --target x86_64-unknown-linux-gnu --all-targets` is clean, engine included
  (on `kde-plasma`, against PipeWire 1.6.9's headers).

Nothing here has run on real hardware, and steamcmd only ran as the test stand-in.

## Build and run

`Cargo.toml` takes apolline-scene from `../apolline-scene`, so put both checkouts on the branch:

```bash
cd ~/src/apolline-scene && git fetch && git switch fix/review-findings
cd ~/src/apolline-wall && git fetch && git switch fix/review-findings
cargo test --workspace && cargo build --release --workspace
systemctl --user stop apolline-wall
./target/release/apolline-wall daemon
```

To test the package instead:
- Edit `packaging/arch/PKGBUILD` locally: the first source must be
  `$_name::git+$url.git#branch=fix/review-findings`, otherwise makepkg builds `main`.
- The apolline-scene source is already pinned at the tip of its branch.
- Don't commit that edit.

## What changed, and how to see each

1. **A paused monitor stops drawing a shared scene.**
   - **Before:** when several monitors showed one moving scene, pausing one of them didn't stop
     it there. `status` said `paused: true`, yet that monitor kept animating and spending GPU.
   - **Now:** it keeps its last frame while the others go on.
   - **Check:**
     1. Put the same scene on two monitors, then pause one: `apolline-wall pause --output <name>`, or on
        X11 cover it with a fullscreen window.
     2. It should freeze while the other moves on, and GPU use should drop. `nvidia-smi` or
        `intel_gpu_top` will show it.
     3. Resume it with `apolline-wall resume --output <name>`, and it should catch up with the other.
2. **A portrait monitor gets a sharp scene.**
   - **Before:** the canvas always had the scene's own shape, held to the monitors' pixel count.
     A portrait 1080×1920 monitor showing a 16:9 scene got a 1920×1080 canvas and showed its
     middle strip stretched 1.78×.
   - **Now:** the canvas is only the part of the scene the monitors show, so that monitor gets a
     1080×1920 one.
   - **Check:**
     1. Show a detailed scene, with text or fine particles, on the rotated monitor alone, in
        `cover`. It should be as sharp as on the landscape one.
     2. Also try `contain` and `blur`: they show the whole scene, as before.
     3. Move the pointer over a parallax scene. Layers should follow the pointer where it is,
        not drift off to one side.
   - **Expected, not a bug:** with a landscape and a portrait monitor sharing one scene, the
     portrait one is still about 1.26× soft. The canvas stays within the two monitors' pixels.
3. **Canvases refit when a monitor leaves a scene.**
   - **Before:** after the portrait monitor switched to a picture, the landscape one kept
     rendering the large two-monitor canvas.
   - **Now:** it shrinks at once.
   - **Check:** two monitors on one scene, then switch one to a picture. The other should keep
     drawing smoothly with no flash, and GPU use should drop.
4. **Project folders are read again only when they change.**
   - **Before:** deciding what a Workshop folder is read its `project.json` for every monitor on
     every refresh.
   - **Now:** it is read once, and again only when the file's change time moves.
   - **Check:**
     1. Workshop videos and scenes still open as what they are.
     2. Change a scene's properties in the app; the scene should update.
     3. Resizing or rescaling a monitor shouldn't stutter.
5. **A failed poster falls back to the video file.**
   - **Before:** when the poster frame of a Workshop video folder couldn't be saved, Apolline
     was handed the folder.
   - **Now:** it gets the folder's video file.
   - **Check:** hard to force. If the log ever shows "poster frame:", the poster in `status`
     should be a `.mp4`-like file, not a folder.
6. **steamcmd signs in from a private script.**
   - **Before:** `+login <account>` put the Steam account name on steamcmd's command line, where
     other users could read it in `/proc`.
   - **Now:** steamcmd gets `+runscript <root>/login.txt`, a 0600 file that is removed when the
     run ends.
   - **Check with the real steamcmd:**
     1. In the app, sign in and download one Workshop item. The password and Steam Guard prompts
        should appear as before, and the item should download.
     2. During the run, `pgrep -a steamcmd` should show no account name.
     3. After the run, `login.txt` should be gone.
   - **If the real steamcmd ignores `+runscript`:** this is the change to roll back first.
7. **A test no longer fails when the temp folder goes through a symlink.**
   - **Check:** `cargo test -p apolline-wall-protocol` passes even with
     `TMPDIR=/some/symlinked/dir`. makepkg's `check()` runs it.

8. **Scenes hear what's playing.**
   - **Before:** audio-reactive scenes (bars, rings, emitters that listen) ran as if in silence.
   - **Now:** while an unpaused monitor shows a scene that listens, the daemon runs
     `parec --device=@DEFAULT_MONITOR@`. It turns what parec hears into 64 bands a side and gives
     them to the renderer each frame. parec stops when no such scene is shown or all are paused.
   - **Needs:** `parec`, from `libpulse` on Arch. It works on PipeWire through pipewire-pulse.
     Without it the log says so once, and scenes stay silent.
   - **Check:**
     1. Show an audio visualiser scene and play music. Bars should follow it: bass on the
        left, treble on the right. The left and right channels are told apart.
     2. While it shows, `pgrep -a parec` should find one parec.
     3. Pause the monitor or switch to a picture, and parec should be gone.
     4. Stop the music, and the bars should fall back smoothly within a fraction of a second.
   - **Guesses to judge by eye:**
     - the bands run from 30 Hz to 16 kHz, spread evenly in pitch;
     - loudness is on a 60 dB scale.

     If a scene looks too quiet, too busy, or crowded at one end compared with Wallpaper Engine
     on Windows, say which. The numbers are at the top of `src/spectrum.rs`.

9. **KDE Plasma 6.** Branch `kde-plasma`.
   - **Before:** Plasma's desktop and the wallpaper both sat on the background layer. Whichever
     mapped last was on top: either the wallpaper didn't show, or it covered the desktop and
     took its clicks.
   - **Now:** when `XDG_CURRENT_DESKTOP` names `KDE`, the wallpaper goes on the bottom layer,
     over Plasma's desktop and under windows and panels, and clicks pass through it.
   - **Check, in a Plasma 6 Wayland session:**
     1. `systemctl --user start apolline-wall`, then `apolline-wall set <a video>`. It shows on
        every monitor, under windows, and the panel stays on top.
     2. Right-click the desktop: Plasma's desktop menu opens. Drag on the desktop: the selection
        rectangle works, even though you can't see it under the wallpaper.
     3. Restart plasmashell (`systemctl --user restart plasma-plasmashell`): the wallpaper stays
        on top of the desktop.
     4. Maximise a window over it, then make it fullscreen. GPU use should drop to near zero
        once the wallpaper is covered.
     5. Lock the screen and unlock: the wallpaper pauses and resumes.
   - **Expected, not bugs:** desktop icons and widgets are hidden under the wallpaper; parallax
     scenes don't follow the pointer.
   - **If the wallpaper covers a panel or a window,** or still doesn't show, run
     `echo $XDG_CURRENT_DESKTOP` in the session and inside `systemctl --user show-environment`,
     and tell me both.

10. **The Plasma wallpaper plugin.** Branch `kde-plasma`.
    - **What it is:** `plasma/io.github.pisosenie.apolline-wall`, a Plasma 6 wallpaper type.
      It runs `apolline-wall feed <connector>` and plays the PipeWire stream the daemon then
      draws that monitor into, so Plasma's icons and widgets stay over the wallpaper.
    - **Set up:**
      1. Install the package built from this branch (see "Build and run": the PKGBUILD edit,
         then `makepkg -si`). The plugin runs `apolline-wall` from plasmashell's `PATH`, so
         `target/release` alone isn't enough.
      2. The build now needs `clang` and PipeWire's headers (`libpipewire`, already on any
         Plasma install).
      3. Restart plasmashell once so it finds the new wallpaper type:
         `systemctl --user restart plasma-plasmashell`.
    - **Check:**
      1. Right-click the desktop → Desktop and Wallpaper → Wallpaper type → **apolline-wall**,
         Apply. Within about two seconds the monitor shows what `apolline-wall status` says it
         shows, with desktop icons and widgets on top.
      2. `apolline-wall status` lists a `feed` node for that monitor, and `pw-cli ls Node`
         shows `apolline-wall.<connector>`.
      3. `apolline-wall set <a video>`, then a shader, then a native scene: each one plays
         in the plugin and crossfades as on other desktops.
      4. Pause and resume the monitor: the picture holds, then moves on.
      5. Do it for every monitor. One set to an ordinary Plasma wallpaper keeps the bottom
         layer from item 9.
      6. Switch the wallpaper type back to Image: Plasma's picture shows with its icons, and
         nothing of ours covers it. `status` still lists the `feed` until the daemon restarts.
      7. `systemctl --user restart apolline-wall` while the plugin shows: the picture comes
         back within a few seconds on its own.
      8. Change the monitor's resolution or scale: the picture comes back at the new size,
         with the same `feed` node.
      9. Maximise a window on that monitor: `apolline-wall status` says it's paused,
         "Covered by a window", and GPU use drops. Restore the window, or press Show Desktop
         (Meta+D): it plays again. A maximised window on another virtual desktop or
         activity doesn't pause it. `apolline-wall policy --fullscreen false` turns this off.
    - **Cost to watch:** frames are read back from the GPU, so expect more CPU and GPU than on
      niri; note what `top` shows for `apolline-wall` and `plasmashell` with a 4K video. These
      numbers decide whether the readback needs to change.
    - **If it stays black:** run `apolline-wall feed <connector>` by hand and tell me what it
      prints. Then run `journalctl --user -u plasma-plasmashell -b | grep -i -E 'pipewire|apolline'`
      and tell me what that shows.

11. **A folder as a playlist.** Branch `playlist-folders`.
    - **Now:** `apolline-wall playlist FOLDER [--output NAME] [--every SECONDS] [--fit …]` plays
      what the folder holds: pictures, videos, shaders and Wallpaper Engine project folders, in
      name order, hidden files left out. The folder is read again before every step.
    - **Check:**
      1. `mkdir /tmp/walls`, copy in three pictures and a video, then
         `apolline-wall playlist /tmp/walls --every 10`. They come one after another, in name
         order, and `apolline-wall status` says `Playlist 1/4`.
      2. Copy in a fifth file whose name sorts before the one showing: the one showing stays,
         `status` says 5 in all, and the new one comes in its turn.
      3. Delete the one showing: the next step goes to what came after it.
      4. Leave one file only: it still steps every 10 s without flicker, and a file copied in
         then joins.
      5. Rename the folder away: the log says it can't be read, and the playlist keeps playing
         what it had. Rename it back: it carries on from the folder.
      6. `systemctl --user restart apolline-wall`: the folder playlist comes back, reading the
         folder afresh.
      7. `apolline-wall stop-playlist` stops it. An empty folder is refused with a message.
    - **Not yet:** the app has no button for it.

## Merging

1. Once both branches check out, merge apolline-scene's into its `main` first.
2. Then merge this one, then `kde-plasma`, then `playlist-folders`, and re-pin `packaging/arch/PKGBUILD` at the scene
   commit that's on `main`.
3. Push that.
