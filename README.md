# ffxi-wrapper

Runs a private [LandSandBoat](https://github.com/LandSandBoat/server) (Final Fantasy XI) server and its database in Docker. The [desktop app](desktop/README.md), for Linux and Windows, sets it up and runs it for you. It also sets up the game client, [vana-client](https://github.com/akreao/vana-client) (a fork of [lenonk/vana-client](https://github.com/lenonk/vana-client)), and installs the game data from Square Enix's installer.

## Running the server

You need Docker with the Compose plugin (Docker Desktop on Windows).

```sh
cp .env.example .env   # then change the passwords
docker compose up -d
docker compose logs -f connect world map
```

The first start downloads the server image (about a gigabyte), which GitHub builds from [`akreao/lsb`](https://github.com/akreao/lsb) at the commit pinned in `.env`. If there's no published image for that commit, or it can't be downloaded, Compose compiles LandSandBoat on this computer instead, which takes 20 to 60 minutes. On the first start `db-init` imports LSB's database, which takes a few more minutes; later starts only apply what changed.

| Port | Server |
|------|--------|
| 54231 TCP | login (auth) |
| 54230 TCP | login (data) |
| 54001 TCP | login (view) |
| 54230 UDP | game (map server) |
| 54002 TCP | search |

The client creates its account on the login screen (LSB's `ACCOUNT_CREATION` is on by default).

`docker compose down` stops everything; the database stays in `DB_DATA`. To start over with an empty database, run `docker compose down -v` (with the default `DB_DATA`) or delete the `DB_DATA` folder. The [desktop app](desktop/README.md) can back the database up into `backups/` and restore it.

## More

- [Settings](docs/settings.md): every `.env` setting, changing LSB's own settings, and players on other networks.
- [Updating the server and client](docs/updating.md): the server image, the game client, bumping the pinned commits, and the login certificate.
- [Website](docs/website.md): the optional `web` service, and putting it on the internet.
- [Auction house bot](docs/auction-house-bot.md): the optional `ahbot` service.

## How it fits together

- `db` is MariaDB 12.3.
- `db-init` runs LSB's `tools/dbtool.py update` and then sets every zone's address to `PUBLIC_IP` ([`docker/lsb/db-init.sh`](docker/lsb/db-init.sh)). The other LSB services wait for it.
- `meshes` copies the navmeshes from LSB's published meshes image into `DATA_DIR` on the first start.
- `connect`, `search`, `world` and `map` are LSB's servers, all from the same image. One map server runs every zone. Each starts through [`docker/lsb/run.sh`](docker/lsb/run.sh), which links the settings in `overrides/settings/` into place, and for `connect` the login certificate in `DATA_DIR/tls/`.
- `ahbot` and `web` are optional services, each behind its own Compose profile (see [Auction house bot](docs/auction-house-bot.md) and [Website](docs/website.md)).
- The PlayOnline profile server (`xi_profile`) and LSB's HTTP API aren't run.
