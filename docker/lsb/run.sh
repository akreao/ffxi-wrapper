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

# xi_connect makes login.key and login.cert in /server on its first start,
# and a new container would make new ones, which vana-client refuses once
# it has pinned the old certificate. So they're links into /tls (DATA_DIR/tls),
# which outlives the container; xi_connect writes them there the first time.
# Half a pair is dropped so a whole new one is made.
if [ "${1:-}" = /server/xi_connect ] && [ -d /tls ]; then
    if { [ -e /tls/login.key ] && [ ! -e /tls/login.cert ]; } || { [ ! -e /tls/login.key ] && [ -e /tls/login.cert ]; }; then
        rm -f /tls/login.key /tls/login.cert
    fi
    ln -sf /tls/login.key /server/login.key
    ln -sf /tls/login.cert /server/login.cert
    # The key is made readable by its owner only; db-init lets anyone read
    # the certificate, which the desktop app pins.
    umask 077
fi

exec /entrypoint.sh "$@"
