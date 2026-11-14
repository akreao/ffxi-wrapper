#!/bin/sh
# Starts the web service as whoever owns the /data mount, not as root, so the
# files it writes there can be read and replaced from the host (the same idea
# as db-init's chown). A root-owned mount (Docker made the folder) runs as
# "nobody" instead of root.
set -eu

data="${DATA_DIR:-/data}"
mkdir -p "$data"

owner="$(stat -c %u:%g "$data")"
if [ "${owner%%:*}" = "0" ]; then
  owner=65534:65534
fi
# Docker Desktop ignores ownership on bind mounts; the chown may be refused there.
chown "$owner" "$data" 2>/dev/null || true

if [ "$#" -eq 0 ]; then
  set -- serve
fi
exec su-exec "$owner" /web "$@"
