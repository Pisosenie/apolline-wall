# Testing the review fixes

Branch `fix/review-findings`, on top of v0.1.0 (`b9730e5`): the review fixes (`4e6cc5d`), then
audio for scenes. It stays off
`main` until it has run on the Linux machine. It builds with apolline-scene's
`fix/review-findings`, whose own
[docs/TESTING.md](https://github.com/Pisosenie/apolline-scene/blob/fix/review-findings/docs/TESTING.md)
covers the renderer.

Checked so far:
- `cargo test --workspace` passes on the Mac.
- `cargo clippy --target x86_64-unknown-linux-gnu --all-targets` is clean, engine included.

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

## Merging

1. Once both branches check out, merge apolline-scene's into its `main` first.
2. Then merge this one, and re-pin `packaging/arch/PKGBUILD` at the scene commit that's on
   `main`.
3. Push that.
