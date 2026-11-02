#!/bin/sh
# Runs before every LSB service: links the settings files in
# overrides/settings/ (map.lua, login.lua and so on) into /server/settings/,
# where LSB reads them on top of its defaults, then starts the service
# through the image's own entrypoint.
set -eu

for f in /overrides/settings/*.lua; do
    [ -e "$f" ] || continue
    ln -sf "$f" "/server/settings/$(basename "$f")"
done

# SERVER_NAME from .env wins over main.lua; left empty, main.lua (or LSB's
# default) decides.
if [ -n "${SERVER_NAME:-}" ]; then
    export XI_MAIN_SERVER_NAME="$SERVER_NAME"
fi
unset SERVER_NAME

exec /entrypoint.sh "$@"
