# Auction house bot

Back to the [README](../README.md).

The optional `ahbot` service runs [ffxiahbot](https://github.com/AdamGagorik/ffxiahbot) (MIT): it buys what players put up for sale and keeps the auction house stocked from a price list of retail prices. It's off unless `.env` has `COMPOSE_PROFILES=ahbot`; its settings are the `AHBOT_*` lines in `.env.example`, and the desktop app's **Auction house** screen edits them. Its log, and a price list of your own (`AHBOT_ITEMS=custom`), live in `DATA_DIR/ahbot/`. `docker compose run --rm --no-deps ahbot refill` stocks it now; `... ahbot clear` takes the bot's own listings down.

The [`Auction house bot image`](../.github/workflows/ahbot-image.yml) workflow builds it at `AHBOT_COMMIT` and publishes `ghcr.io/akreao/ffxi-wrapper/ahbot:<commit>`; without one, the stack builds it from [`docker/ahbot/`](../docker/ahbot/Dockerfile). Change the pin in `.env.example` and `compose.yaml` together.
