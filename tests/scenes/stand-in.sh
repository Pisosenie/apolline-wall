#!/usr/bin/env bash
# Stands in for linux-wallpaperengine in tools/headless.sh: a flat colour on the monitor it is
# given, or the way it fails when a project has no assets.
while [ $# -gt 0 ]; do
    case $1 in
        --screen-root) output=$2; shift 2 ;;
        --bg) project=$2; shift 2 ;;
        *) shift ;;
    esac
done
if [ -e "$project/crash" ]; then
    echo "Cannot find a valid assets folder" >&2
    exit 1
fi
exec swaybg -o "$output" -c '#1f8f4f'
