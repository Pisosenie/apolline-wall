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
mkfifo "$fifo"
printf 'output HEADLESS-1 resolution 1440x900 scale 1\ndefault_border none\n' > "$conf"
before=$(sockets)
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway -c "$conf" > /dev/null 2>&1 &
sway=$!
trap 'kill $sway 2> /dev/null; rm -f "$conf" "$fifo"' EXIT

for _ in $(seq 50); do
    display=$(comm -13 <(echo "$before") <(sockets) | head -1)
    [ -n "$display" ] && break
    sleep 0.1
done
export WAYLAND_DISPLAY=${display:?sway did not start}

events=$out/events.jsonl
LIBGL_ALWAYS_SOFTWARE=1 "$target/release/apolline-wall" < "$fifo" > "$events" 2> "$out/stderr.log" &
wall=$!
exec 3> "$fifo"

send() { echo "$1" >&3; }
ready() {
    for _ in $(seq 100); do
        [ "$(grep -c '"ready"' "$events" || true)" -ge "$1" ] && return
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
