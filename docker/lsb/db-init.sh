#!/bin/sh
# Brings LSB's database up to date, then sets the address players are sent to.
#
# dbtool remembers which commit the database was last updated to in
# tools/config.yaml. That file lives in DATA_DIR/dbtool, so a later
# start only applies what changed since (or nothing at all).
set -eu

case "${PUBLIC_IP:-}" in
    *[!0-9.]* | "")
        echo "PUBLIC_IP must be an IPv4 address, got '${PUBLIC_IP:-}'" >&2
        exit 1
        ;;
esac

export MYSQL_PWD="$XI_NETWORK_SQL_PASSWORD"
sql() {
    mariadb --skip-ssl-verify-server-cert -h"$XI_NETWORK_SQL_HOST" -P"$XI_NETWORK_SQL_PORT" \
        -u"$XI_NETWORK_SQL_LOGIN" "$XI_NETWORK_SQL_DATABASE" -N -e "$1"
}

# A database that was deleted (docker compose down -v, or a new db-data
# volume) needs a full import, whatever the old state says.
if [ -z "$(sql "SHOW TABLES LIKE 'zone_settings'")" ]; then
    echo "Empty database: importing LandSandBoat's tables. This takes a few minutes."
    rm -f /state/config.yaml
fi
ln -sf /state/config.yaml /server/tools/config.yaml

cd /server
# The desktop app leaves this table in a database it restores from a
# backup. dbtool's record describes the database from before the restore,
# so this one gets a full update: LSB's game data is imported again and
# every migration checked, while accounts and characters are kept.
if [ -n "$(sql "SHOW TABLES LIKE 'ffxi_wrapper_restored'")" ]; then
    echo "Restored from a backup: bringing the whole database up to date. This takes a few minutes."
    python tools/dbtool.py update full
    sql "DROP TABLE ffxi_wrapper_restored"
else
    python tools/dbtool.py update
fi

# Clients are sent this address for the map server.
sql "UPDATE zone_settings SET zoneip = '$PUBLIC_IP'"
echo "Database ready. Players are sent to $PUBLIC_IP."
