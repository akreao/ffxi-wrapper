#!/bin/sh
# Runs ffxiahbot against the stack's database. The first argument picks
# what it does:
#   broker  buys what players list and keeps the bot's listings stocked (default)
#   refill  stocks the bot's listings once, now
#   clear   removes the bot's own listings (players' listings stay)
# Its settings come from .env (AHBOT_*); see .env.example. Its log, and
# the price list when AHBOT_ITEMS is "custom", are kept in DATA_DIR/ahbot.
set -eu

mode="${1:-broker}"
work=/ahbot
mkdir -p "$work"
cd "$work"

case "${AHBOT_ITEMS:-items75}" in
  items75) csv=/opt/ahbot/items75.csv ;;
  items) csv=/opt/ahbot/items.csv ;;
  custom)
    csv="$work/items.csv"
    if [ ! -f "$csv" ]; then
      # Start the user's own list from the level 75 one.
      cp /opt/ahbot/items75.csv "$csv"
      chmod a+rw "$csv"
      echo "Made $csv from the level 75 list; edit it to change prices and stock."
    fi
    ;;
  *) echo "AHBOT_ITEMS must be items75, items or custom, not '${AHBOT_ITEMS}'." >&2; exit 1 ;;
esac

# Written outside DATA_DIR: it holds the database password.
python - <<'PY'
import os
import yaml

def number(key, default):
    value = os.environ.get(key) or default
    return max(1, int(value))

config = {
    "name": os.environ.get("AHBOT_NAME") or "M.H.M.U.",
    "tick": number("AHBOT_TICK", 30),
    "restock": number("AHBOT_RESTOCK", 3600),
    "use_buying_rates": False,
    "use_selling_rates": False,
    "hostname": "db",
    "database": "xidb",
    "username": "xiadmin",
    "password": os.environ["DB_PASSWORD"],
    "port": 3306,
    "fail": False,
}
with open("/tmp/config.yaml", "w") as f:
    yaml.safe_dump(config, f)
PY

flag() { if [ "$2" = "no" ]; then echo "--no-$1"; else echo "--$1"; fi; }

touch "$work/ahbot.log"
chmod a+rw "$work/ahbot.log"
run() { exec ffxiahbot --logfile "$work/ahbot.log" "$@"; }

case "$mode" in
  broker)
    # Stocks at once instead of waiting a whole restock interval first.
    run broker --config /tmp/config.yaml --inp-csv "$csv" \
      "$(flag buy-items "${AHBOT_BUY:-yes}")" "$(flag sell-items "${AHBOT_SELL:-yes}")" \
      --restock-immediately
    ;;
  refill) run refill --config /tmp/config.yaml --inp-csv "$csv" --no-prompt ;;
  clear) run clear --config /tmp/config.yaml --no-prompt ;;
  *) echo "Unknown mode '$mode': use broker, refill or clear." >&2; exit 1 ;;
esac
