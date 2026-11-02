#!/usr/bin/env bash
# Drive apolline-wall under a throwaway headless sway and screenshot each step.
# usage: tools/headless.sh OUT_DIR   (Linux, needs sway and grim)
set -euo pipefail

out=$(realpath -m "$1")
mkdir -p "$out"
root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
walls=$root/tests/pictures
sockets() { ls "$runtime" | grep -E '^wayland-[0-9]+$' || true; }

cargo build --manifest-path "$root/Cargo.toml" --release --quiet

conf=$(mktemp)
fifo=$(mktemp -u)
scenes=$(mktemp -d)
mkfifo "$fifo"
printf 'output HEADLESS-1 resolution 1440x900 scale 1\ndefault_border none\n' > "$conf"
before=$(sockets)
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway -c "$conf" > /dev/null 2>&1 &
sway=$!
trap 'kill $sway 2> /dev/null; rm -rf "$conf" "$fifo" "$scenes"' EXIT

for _ in $(seq 50); do
    display=$(comm -13 <(echo "$before") <(sockets) | head -1)
    [ -n "$display" ] && break
    sleep 0.1
done
export WAYLAND_DISPLAY=${display:?sway did not start}

events=$out/events.jsonl
export APOLLINE_WALL_SCENE_RENDERER=$root/tests/scenes/stand-in.sh
LIBGL_ALWAYS_SOFTWARE=1 "$target/release/apolline-wall" < "$fifo" > "$events" 2> "$out/stderr.log" &
wall=$!
exec 3> "$fifo"

send() { echo "$1" >&3; }
shown() { grep -c '"ready"' "$events" || true; }
differ() { cmp -s "$out/$1" "$out/$2" && echo "$3: no" || echo "$3: yes"; }
ready() {
    for _ in $(seq 200); do
        [ "$(shown)" -ge "$1" ] && return
        sleep 0.05
    done
    echo "no ready event #$1" >&2
}

send '{"show":{"source":"'"$walls"'/fog.jpg"}}'
ready 1
sleep 0.5
grim "$out/1-fog.png"

send '{"show":{"source":"'"$walls"'/ember.jpg","transition_ms":3000}}'
ready 2
sleep 1.5
grim "$out/2-halfway.png"
sleep 2
grim "$out/3-ember.png"

send '{"pause":{"paused":true}}'
send '{"show":{"source":"'"$walls"'/night.jpg","transition_ms":3000}}'
ready 3
sleep 0.3
grim "$out/4-night-while-paused.png"

if command -v ffmpeg > /dev/null && ldconfig -p | grep 'libmpv\.so' > /dev/null; then
    clip=$out/clip.mp4
    ffmpeg -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -t 2 \
        -c:v libx264 -pix_fmt yuv420p "$clip"
    send '{"pause":{"paused":false}}'
    send '{"show":{"source":"'"$clip"'","transition_ms":500}}'
    ready 4
    sleep 0.8
    grim "$out/5-video.png"
    sleep 0.3
    grim "$out/6-video.png"
    differ 5-video.png 6-video.png "the video moves"
    send '{"pause":{"paused":true}}'
    sleep 0.3
    grim "$out/7-video-paused.png"
    sleep 0.5
    grim "$out/8-video-paused.png"
    differ 7-video-paused.png 8-video-paused.png "a paused video moves"
    send '{"pause":{"paused":false}}'
    send '{"show":{"source":"'"$walls"'/fog.jpg","transition_ms":2000}}'
    ready 5
    sleep 1
    grim "$out/9-video-to-fog.png"
else
    echo "no ffmpeg or libmpv, skipping videos" >&2
fi

send '{"pause":{"paused":false}}'
before=$(shown)
send '{"show":{"source":"'"$root"'/tests/shaders/hills.frag","transition_ms":500}}'
ready $((before + 1))
sleep 0.8
grim "$out/s1-shader.png"
sleep 0.5
grim "$out/s2-shader.png"
differ s1-shader.png s2-shader.png "the shader moves"
send '{"uniforms":{"accent":"#ff6a3d"}}'
send '{"pause":{"paused":true}}'
sleep 0.3
grim "$out/s3-shader-ember.png"
sleep 0.5
grim "$out/s4-shader-ember.png"
differ s3-shader-ember.png s4-shader-ember.png "a paused shader moves"
differ s2-shader.png s3-shader-ember.png "a new accent shows while paused"
send '{"pause":{"paused":false}}'
send '{"show":{"source":"'"$walls"'/fog.jpg","transition_ms":2000}}'
ready $((before + 2))
sleep 1
grim "$out/s5-shader-to-fog.png"

if command -v swaybg > /dev/null; then
    mkdir "$scenes/forest" "$scenes/broken"
    echo '{"title":"Forest","type":"scene","preview":"preview.jpg"}' > "$scenes/forest/project.json"
    cp "$walls/night.jpg" "$scenes/forest/preview.jpg"
    echo '{"type":"scene"}' > "$scenes/broken/project.json"
    touch "$scenes/broken/crash"
    before=$(shown)
    send '{"show":{"source":"'"$scenes"'/forest"}}'
    ready $((before + 1))
    grim "$out/c1-scene.png"
    differ s5-shader-to-fog.png c1-scene.png "the scene shows"
    renderer=$(pgrep -P $wall -x swaybg || true)
    send '{"pause":{"paused":true}}'
    sleep 0.3
    echo "a paused scene's renderer: $(awk '/^State/ {print $3}' /proc/${renderer:?no renderer under the daemon}/status)"
    send '{"pause":{"paused":false}}'
    send '{"show":{"source":"'"$walls"'/ember.jpg"}}'
    ready $((before + 2))
    sleep 0.5
    grim "$out/c2-scene-to-ember.png"
    kill -0 "$renderer" 2> /dev/null && echo "the renderer outlived its scene" >&2
    send '{"show":{"source":"'"$scenes"'/broken"}}'
    ready $((before + 3))
    sleep 0.5
    grim "$out/c3-after-crash.png"
    differ c2-scene-to-ember.png c3-after-crash.png "a crashed scene changes the desktop"
else
    echo "no swaybg to stand in for linux-wallpaperengine, skipping scenes" >&2
fi

send '{"show":{"source":"/nowhere/missing.jpg"}}'
send '{"lights":{"on":true}}'
sleep 0.5

exec 3>&-
for _ in $(seq 40); do
    kill -0 $wall 2> /dev/null || break
    sleep 0.05
done
kill -0 $wall 2> /dev/null && echo "still running after stdin closed" >&2
cat "$events"
ls "$out"
