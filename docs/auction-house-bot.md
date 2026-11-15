# Auction house bot

Back to the [README](../README.md).

The optional `ahbot` service runs [ffxiahbot](https://github.com/AdamGagorik/ffxiahbot) (MIT). It buys what players put up for sale and keeps the auction house stocked from a price list of retail prices.

- It is off unless `.env` has `COMPOSE_PROFILES=ahbot`.
- Its settings are the `AHBOT_*` lines in `.env.example`. The desktop app's **Auction house** screen edits them.
- Its log lives in `DATA_DIR/ahbot/`. So does a price list of your own (`AHBOT_ITEMS=custom`).
- To stock the auction house now, run `docker compose run --rm --no-deps ahbot refill`.
- To take down the bot's own listings, run `docker compose run --rm --no-deps ahbot clear`.

The [`Auction house bot image`](../.github/workflows/ahbot-image.yml) workflow builds the bot at `AHBOT_COMMIT` and publishes `ghcr.io/akreao/ffxi-wrapper/ahbot:<commit>`. Without a published image, the stack builds it from [`docker/ahbot/`](../docker/ahbot/Dockerfile). Change the pin in `.env.example` and `compose.yaml` together.
