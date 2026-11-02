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

exec /entrypoint.sh "$@"
