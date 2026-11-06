# ffxi-wrapper

Runs a private [LandSandBoat](https://github.com/LandSandBoat/server) (Final Fantasy XI) server and its database in Docker. The [desktop app](desktop/README.md), for Linux and Windows, sets it up and runs it for you. It also sets up the game client, [vana-client](https://github.com/akreao/vana-client) (a fork of [lenonk/vana-client](https://github.com/lenonk/vana-client)), and installs the game data from Square Enix's installer.

## Running the server stack

You need Docker with the Compose plugin (Docker Desktop on Windows).

```sh
cp .env.example .env   # then change the passwords
docker compose up -d
docker compose logs -f connect world map
```

The first start downloads the server image (about a gigabyte), which GitHub builds from [`akreao/lsb`](https://github.com/akreao/lsb) at the commit pinned in `.env` (see [Server image](#server-image)). If there's no published image for that commit, or it can't be downloaded, Compose compiles LandSandBoat on this computer instead, which takes 20 to 60 minutes. On the first start `db-init` imports LSB's database, which takes a few more minutes; later starts only apply what changed.

| Port | Server |
|------|--------|
| 54231 TCP | login (auth) |
| 54230 TCP | login (data) |
| 54001 TCP | login (view) |
| 54230 UDP | game (map server) |
| 54002 TCP | search |

The client creates its account on the login screen (LSB's `ACCOUNT_CREATION` is on by default).

### Settings in `.env`

- `DATA_DIR` is where the server keeps its files: dbtool's record of the database version (`dbtool/`) and the navmeshes (`navmeshes/`, `ximeshes/`). It's `./data` by default; point it at another drive to keep them there.
- `DB_DATA` is where the database lives. The default, `db-data`, is a Docker volume, which works everywhere and is the reliable choice on Windows (Docker Desktop's Settings, Resources, Disk image location moves it to another drive). On Linux it can be a folder instead, such as `./data/db`, but only on a Linux filesystem (ext4, btrfs, xfs): MariaDB needs Linux file ownership and locking, which exFAT and NTFS don't give it.
- `PUBLIC_IP` is the address clients are sent for the map server. Keep `127.0.0.1` when the client runs on this computer; use the computer's LAN or VPN address for players on other computers. It's applied by `db-init` on each start (`docker compose up -d`).
- `BIND_IP`, when set to one of this computer's IPv4 addresses, makes the ports listen on that address only. Docker's port rules go around ufw and firewalld, so this, not the host firewall, keeps the server off other networks.
- `SERVER_NAME` is the server name shown in the client. Left empty, `overrides/settings/main.lua` (or LSB's default, Nameless) decides.
- `LSB_REPO` and `LSB_COMMIT` choose the LandSandBoat source. The stack uses `ghcr.io/akreao/ffxi-wrapper/lsb:<LSB_COMMIT>` when GitHub has built it, and compiles that commit here otherwise.
- `LSB_IMAGE`, when set, uses another image instead (such as `ghcr.io/landsandboat/server:latest`, upstream LSB).

### Server image

The [`LSB server image`](.github/workflows/lsb-image.yml) workflow compiles LandSandBoat from `LSB_REPO` at the `LSB_COMMIT` pinned in `.env.example` and publishes it as `ghcr.io/akreao/ffxi-wrapper/lsb:<commit>` (and `:latest`), for linux/amd64. It runs when that pin changes on `main`, and by hand from the Actions tab, where it can also build any other commit. A commit that's already published isn't built again.

To move the server to a newer LSB commit, change `LSB_COMMIT` in both `.env.example` and `compose.yaml` (its default), add the new commit to `APP_LSB_PINS` in `desktop/src-tauri/src/stack.rs` (a test fails until you do), and merge. `scripts/bump-pins.py --lsb <commit>` does all three (see below). The app leaves `LSB_COMMIT` empty in its `.env`, so it always uses the pin it shipped with; once the workflow has published the image, `docker compose up -d` pulls it and recreates the servers.

To compile on this computer anyway, run `docker compose build db-init` (or `docker compose up -d --build`).

### Game client

The [`Game client`](.github/workflows/client.yml) workflow builds vana-client from `CLIENT_REPO` at the `CLIENT_COMMIT` pinned in `.env.example`, for Linux (in the Steam Runtime SDK, so it runs on any current distribution) and Windows, and publishes both as a release of this repository tagged `client-<first 12 characters of the commit>`. Each file has its SHA-256 beside it (`<file>.sha256`). The desktop app downloads the build for its platform from there and installs it only if it matches that checksum. Like the server image, it runs when the pin changes on `main` and by hand from the Actions tab; a published commit isn't built again unless asked to.

To move to a newer client, push the change to the fork, put its full commit SHA in `CLIENT_COMMIT` in `.env.example` and merge. Once the release is published, the app offers the update.

### Bumping the pins

`scripts/bump-pins.py` updates every file that pins either commit. Give it a full or short SHA, a tag or a branch name; it looks it up on GitHub (in `LSB_REPO` / `CLIENT_REPO`), writes the full SHA and prints a diff:

```sh
scripts/bump-pins.py --lsb 1a2b3c4d --client v1.2.0
scripts/bump-pins.py --client <sha> --dry-run   # show the diff, write nothing
```

For LSB it changes `.env.example`, all of `compose.yaml`'s defaults and adds the new pin to `APP_LSB_PINS`; for the client, `.env.example`. It warns when the commit isn't on the repository's default branch (a pin there could be lost if its branch is reset). `--offline` skips the lookup and takes full SHAs only; set `GITHUB_TOKEN` if you hit GitHub's rate limit. It needs Python 3.10 or later.

vana-client pins each server's login certificate and refuses a different one later. `connect` keeps the certificate it makes on its first start in `DATA_DIR/tls/` (`login.cert` and `login.key`), so recreating the container or moving to a new server image keeps it. The desktop app pins it in its own client before each start, so that client never has to trust it on first use. Players on other computers can pin it with `vana-client trust-server SERVER --port 54231 --fingerprint ...`, using the fingerprint from `openssl x509 -in DATA_DIR/tls/login.cert -noout -fingerprint -sha256`. Deleting `DATA_DIR/tls/` makes a new certificate on the next start, which every player's client then has to trust again.

### Players on other networks

The server's ports include UDP 54230 (the game itself), so web tunnels such as Cloudflare Tunnel can't carry it. A private network such as Tailscale can: set `PUBLIC_IP` to this computer's Tailscale address, share the computer with each player in Tailscale, and players point vana-client at that address. The desktop app's **Play with friends** screen walks through it.

### Changing LSB's settings

Any LSB setting can be set as an environment variable, `XI_<FILE>_<SETTING>` (for example `XI_MAP_EXP_RATE`), in the `x-lsb` environment in `compose.yaml`. For bigger changes, put a copy of one of LSB's settings files (`settings/default/map.lua` and the rest in the LSB repo) in [`overrides/settings/`](overrides/settings/README.md), keep only the values you change, and restart: `docker compose up -d --force-recreate connect search world map`.

### Auction house bot

The optional `ahbot` service runs [ffxiahbot](https://github.com/AdamGagorik/ffxiahbot) (MIT): it buys what players put up for sale and keeps the auction house stocked from a price list of retail prices. It's off unless `.env` has `COMPOSE_PROFILES=ahbot`; its settings are the `AHBOT_*` lines in `.env.example`, and the desktop app's **Auction house** screen edits them. Its log, and a price list of your own (`AHBOT_ITEMS=custom`), live in `DATA_DIR/ahbot/`. `docker compose run --rm --no-deps ahbot refill` stocks it now; `... ahbot clear` takes the bot's own listings down.

The [`Auction house bot image`](.github/workflows/ahbot-image.yml) workflow builds it at `AHBOT_COMMIT` and publishes `ghcr.io/akreao/ffxi-wrapper/ahbot:<commit>`; without one, the stack builds it from [`docker/ahbot/`](docker/ahbot/Dockerfile). Change the pin in `.env.example` and `compose.yaml` together.

### Stopping and wiping

`docker compose down` stops everything; the database stays in `DB_DATA`. To start over with an empty database, run `docker compose down -v` (with the default `DB_DATA`) or delete the `DB_DATA` folder. The [desktop app](desktop/README.md) can back the database up into `backups/` and restore it.

## How it fits together

- `db` is MariaDB 12.3.
- `db-init` runs LSB's `tools/dbtool.py update` and then sets every zone's address to `PUBLIC_IP` ([`docker/lsb/db-init.sh`](docker/lsb/db-init.sh)). The other LSB services wait for it.
- `meshes` copies the navmeshes from LSB's published meshes image into `DATA_DIR` on the first start.
- `connect`, `search`, `world` and `map` are LSB's servers, all from the same image. One map server runs every zone. Each starts through [`docker/lsb/run.sh`](docker/lsb/run.sh), which links the settings in `overrides/settings/` into place, and for `connect` the login certificate in `DATA_DIR/tls/`.
- The PlayOnline profile server (`xi_profile`) and LSB's HTTP API aren't run.
