#!/usr/bin/env bash
# Stands in for steamcmd in the app's tests: the same arguments, its prompts and lines, and a
# fake Workshop item where steamcmd would put a real one. It knows one made-up account,
# `stand_in`, with the password `stand-in-password` and the Steam Guard code `QX7RM`.
while [ $# -gt 0 ]; do
    case $1 in
        +force_install_dir) root=$2; shift 2 ;;
        +runscript) account=$(sed -n 's/^login //p' "$2"); shift 2 ;;
        +licenses_for_app) licenses=1; shift 2 ;;
        +workshop_download_item) item=$3; shift 3 ;;
        *) shift ;;
    esac
done
echo "Redirecting stderr to '$root/logs/stderr.txt'"
echo "Loading Steam API...OK"
if [ -e "$root/.signed-in" ]; then
    echo "Logging in using cached credentials."
else
    echo "Logging in user '$account' [U:1:0] to Steam Public..."
    printf 'password: '
    read -rs password
    echo
    if [ "$account" != stand_in ] || [ "$password" != stand-in-password ]; then
        echo "FAILED (Invalid Password)"
        exit 5
    fi
    printf 'Steam Guard code: '
    read -r code
    if [ "$code" != QX7RM ]; then
        echo "FAILED (Two-factor code mismatch)"
        exit 5
    fi
    touch "$root/.signed-in"
fi
echo "Waiting for user info...OK"
if [ -n "$licenses" ]; then
    echo "License packageid 69373:"
    echo " - State   : Active( flags 0 ) - Purchased : Sat Oct  1 2022"
    echo " - Apps    : 431960,  (1 in total)"
fi
if [ -n "$item" ]; then
    if [ "$item" = 13 ]; then
        echo "ERROR! Download item 13 failed (Access Denied)."
        exit 6
    fi
    downloading=$root/steamapps/workshop/downloads/431960/$item
    mkdir -p "$downloading"
    head -c 4096 /dev/zero > "$downloading/clip.mp4"
    printf '{"title":"Stand-in %s","type":"video","file":"clip.mp4"}' "$item" > "$downloading/project.json"
    sleep 1.2
    content=$root/steamapps/workshop/content/431960
    mkdir -p "$content"
    rm -rf "${content:?}/$item"
    mv "$downloading" "$content/$item"
    printf '"AppWorkshop"\n{\n\t"WorkshopItemsInstalled"\n\t{\n\t\t"%s"\n\t\t{\n\t\t\t"timeupdated"\t\t"1"\n\t\t}\n\t}\n}\n' "$item" \
        > "$root/steamapps/workshop/appworkshop_431960.acf"
    echo "Success. Downloaded item $item to \"$content/$item\" (4096 bytes) "
fi
