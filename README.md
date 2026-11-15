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

The [`Checks`](.github/workflows/checks.yml) workflow runs on every pull request. It tests `bump-pins.py` (`python3 -m unittest discover -s scripts`), fails if `.env.example`, `compose.yaml` and `APP_LSB_PINS` disagree on a pin, and checks that `compose.yaml` parses, the stack's shell scripts pass ShellCheck, the UI's JavaScript parses and the workflows pass actionlint.

vana-client pins each server's login certificate and refuses a different one later. `connect` keeps the certificate it makes on its first start in `DATA_DIR/tls/` (`login.cert` and `login.key`), so recreating the container or moving to a new server image keeps it. The desktop app pins it in its own client before each start, so that client never has to trust it on first use. Players on other computers can pin it with `vana-client trust-server SERVER --port 54231 --fingerprint ...`, using the key fingerprint the desktop app shows under Play with friends, or from `openssl x509 -in DATA_DIR/tls/login.cert -noout -pubkey | openssl pkey -pubin -outform DER | openssl dgst -sha256` (the key's, not `-fingerprint`, which hashes the whole certificate). Deleting `DATA_DIR/tls/` makes a new certificate on the next start, which every player's client then has to trust again.

### Players on other networks

The server's ports include UDP 54230 (the game itself), so web tunnels such as Cloudflare Tunnel can't carry it. A private network such as Tailscale can: set `PUBLIC_IP` to this computer's Tailscale address, share the computer with each player in Tailscale, and players point vana-client at that address. The desktop app's **Play with friends** screen walks through it.

### Changing LSB's settings

Any LSB setting can be set as an environment variable, `XI_<FILE>_<SETTING>` (for example `XI_MAP_EXP_RATE`), in the `x-lsb` environment in `compose.yaml`. For bigger changes, put a copy of one of LSB's settings files (`settings/default/map.lua` and the rest in the LSB repo) in [`overrides/settings/`](overrides/settings/README.md), keep only the values you change, and restart: `docker compose up -d --force-recreate connect search world map`.

### Auction house bot

The optional `ahbot` service runs [ffxiahbot](https://github.com/AdamGagorik/ffxiahbot) (MIT): it buys what players put up for sale and keeps the auction house stocked from a price list of retail prices. It's off unless `.env` has `COMPOSE_PROFILES=ahbot`; its settings are the `AHBOT_*` lines in `.env.example`, and the desktop app's **Auction house** screen edits them. Its log, and a price list of your own (`AHBOT_ITEMS=custom`), live in `DATA_DIR/ahbot/`. `docker compose run --rm --no-deps ahbot refill` stocks it now; `... ahbot clear` takes the bot's own listings down.

The [`Auction house bot image`](.github/workflows/ahbot-image.yml) workflow builds it at `AHBOT_COMMIT` and publishes `ghcr.io/akreao/ffxi-wrapper/ahbot:<commit>`; without one, the stack builds it from [`docker/ahbot/`](docker/ahbot/Dockerfile). Change the pin in `.env.example` and `compose.yaml` together.

### Website

The optional `web` service is a website for the server: a home page, **My account** for every game account (its characters, read-only, and a password change), and an admin panel for accounts with the Moderator or Owner website role: a dashboard (who is online, busiest zones, service status, recent actions), searchable Accounts and Characters, Bans, and an Admin log. Staff ban and unban accounts (a ban sets `accounts.status`, keeps its end and reason in `accounts_banned`, and the site lifts timed bans when they run out; it doesn't remove anyone already in game); only owners reset passwords and two-step sign-in, change in-game GM levels and set website roles. Nobody acts on an account whose website role is the same as theirs or higher, and Moderators leave accounts with an in-game GM character to owners, and every change needs a reason where it matters and is written to the log. It's off unless `.env` has `COMPOSE_PROFILES=web` (comma-separated with `ahbot`), and it listens on this computer only, <http://127.0.0.1:54080>. `WEB_BIND_IP` in `.env` opens it to a LAN or Tailscale address of this computer, `WEB_PORT` changes the port. On its own the site speaks plain HTTP on that port; to put it on the internet use the `public` profile below, never this port.

- **Signing in.** Every game account signs in with its login and password (passwords stay LSB's own bcrypt hashes; an account with an old-style hash logs in to the game once first; two-step sign-in is honoured). **Website roles and GM levels are separate.** A website role (Player, Moderator or Owner) belongs to an account and is kept in the site's own database; an owner sets it on the account's page, and only the owner link can take Owner away from someone. A GM level belongs to a character (LSB's `chars.gmlevel`) and only controls in-game GM commands, so giving a character GM 3 for `!goto`, or `!promote` in game, gives no website access. When a site that already ran an older version first starts with this, it copies the roles it used to work out from GM levels (highest level among an account's characters: 5 Owner, 3 and 4 Moderator), once, so nobody loses access. A new or wiped site copies nothing: open the admin panel with the desktop app's owner link and set roles there. The role is read again on every request, so a ban or role change takes effect at once. Failed sign-ins are limited per address and login, and every change needs a CSRF token.
- **Desktop app.** The app's **Website** screen turns it on and off, picks who can reach it and opens the admin panel signed in as the owner (it generates `ADMIN_SECRET` when there is none).
- **The owner link.** `ADMIN_SECRET` (64 hex characters, `openssl rand -hex 32`) signs a one-minute, single-use link, `/auth/owner?t=...`, that signs the owner in without a password. Empty turns it off.
- **Its database user.** The site connects as `xiweb`, which `db-init` creates from `DB_ROOT_PASSWORD` with its own password, `WEB_DB_PASSWORD` (the web container never gets `xiadmin`'s): `SELECT` on the tables its pages read, `UPDATE` on `accounts.password` and `INSERT` on the seven `accounts` columns LSB's own account creation fills, nothing else. Later phases add the grants they need to `docker/lsb/db-init.sh`.
- **Sign-up.** Visitors can create a game account on the site, made exactly as LSB's login server makes one (`LOGIN_CREATE`), but only while the owner has it open (**Invite codes** in the admin panel; closed until switched on, and closed if the site can't read its own settings). LSB's `login.ACCOUNT_CREATION` isn't read: it only covers accounts made from the game's login screen, so switch it off (a settings file or the desktop app's Game settings) to make the site the only way to get an account. A login is 3-16 letters or numbers, a password 8-32 characters. By default a sign-up also needs an invite code, which only the owner makes (**Invite codes** in the admin panel; each is shown once; switch the requirement off there). Limits: 3 sign-ups an hour per address, 30 accounts a day for the whole server. The new account is signed in and taken to **My account**, which shows how to connect.
- **Audit logs.** Staff read LSB's own logs in the admin panel: chat, GM commands, trades, bazaar sales and delivery box sends, filtered by log, chat channel, character or account, dates and text, newest first. Tells are shown to the owner only; the Rules page tells players that the server may log chat. LSB writes these logs only for what the desktop app's Game settings switch on (Chat and moderation: *Log chat to the database* plus each channel, *Log GM commands*, trades, bazaar, delivery box); all are off by default. **Retention:** LSB never deletes from `audit_chat`, `audit_gm`, `audit_trade`, `audit_bazaar` or `audit_dbox`, and the site only reads them (`SELECT`), so they grow for as long as logging is on. The site shows the last 90 days, at most 40 pages per search, runs at most two searches at once and stops a search after 5 seconds; every search (which log and filters, not what it found) goes in the Admin log. To trim old rows, delete them as `xiadmin`, for example `DELETE FROM audit_chat WHERE datetime < NOW() - INTERVAL 180 DAY;`.
- **What it keeps.** The site's own SQLite file (sessions and the admin action log) is `DATA_DIR/web/web.db`; nothing is added to the game database. Backups from the desktop app don't include it yet.
- **The image.** [`docker/web/`](docker/web/Dockerfile) builds the Rust service in [`web/`](web/) with the shared crate [`core/`](core/); the [`Website image`](.github/workflows/web-image.yml) workflow publishes `ghcr.io/akreao/ffxi-wrapper/web:<version>` and tags the commit `web-v<version>`. The version is the one in `web/Cargo.toml`; `compose.yaml` names it twice (the image and the `#web-v<version>` build fallback), and a pull request that changes `web/`, `core/` or `docker/web/` has to bump all three. Its pages share `desktop/ui/tokens.css` (fonts, colours, frames and controls) with the desktop app.

#### Website on the internet

The `public` profile adds a [Caddy](https://caddyserver.com) container (`proxy`) that answers on ports 80 and 443, gets and renews a Let's Encrypt certificate by itself, sends plain-HTTP visitors to HTTPS and passes the rest to `web`; TLS ends on this computer, so nobody else sees players' passwords. Its settings are [`docker/caddy/Caddyfile`](docker/caddy/Caddyfile). What to do:

1. Pick a name, such as `ffxi.example.com`, and make a DNS **A record** for it pointing at this computer's internet address. With Cloudflare keep the record **DNS only** (grey cloud): its proxy would decrypt the traffic again and can't carry the other ports anyway. If your home address changes, step 4 keeps the record current.
2. On the router, forward **TCP 80 and 443** to this computer. (Port 80 is for the certificate check and the redirect; the game's own ports are not part of this.)
3. In `.env`: `COMPOSE_PROFILES=web,public` (plus `ahbot` if you use it), `WEB_DOMAIN=ffxi.example.com` (`WEB_PUBLIC_URL` is worked out from it; set it only to override). Then `docker compose up -d`. The first visit takes a few seconds while the certificate is issued; `docker compose logs proxy` shows how it went. Certificates and the account key are kept in `DATA_DIR/caddy/`: keep that folder private.
4. Optional, for an address that changes: add `ddns` to `COMPOSE_PROFILES` and put a Cloudflare API token in `.env` as `CLOUDFLARE_API_TOKEN`. Create the token with only **Zone > DNS > Edit** on the domain's zone. The `ddns` service ([favonia/cloudflare-ddns](https://github.com/favonia/cloudflare-ddns)) then keeps `WEB_DOMAIN`'s A record on this computer's address, unproxied. The token is read from `.env` only; don't paste it anywhere else.

How the site behaves behind the proxy: it believes the proxy's `X-Forwarded-For` only when the connection comes from the `proxy` container itself (found by name, re-checked every 15 seconds) and then uses its last entry, the address Caddy saw, so a visitor can't pick the address that rate limits, sessions and the admin log record. Caddy answers nothing for another name than `WEB_DOMAIN`, and the site itself refuses one that gets through (421). Over HTTPS the site sends HSTS and `Secure` cookies. While `WEB_PUBLIC_URL` is set, signing in over plain HTTP is refused unless it comes from this computer, a private network address or Tailscale: the direct port `54080` keeps working for the owner and the LAN, not for the internet. Without a domain, Tailscale stays the way in.

Adding to the site: a route is a handler plus one `.route(...)` line in the module for its audience under [`web/src/routes/`](web/src/routes/) (`public`, `session`, `me`, `admin`), a page is a file in [`web/ui/`](web/ui/) plus a line in `routes/pages.rs` and a function in `ui/app.js`, and a query is a method on the `Game` trait in `web/src/game.rs` (with a fake in `web/tests/common/`). Run the tests with `cargo test` in `web/` and `core/`.

### Stopping and wiping

`docker compose down` stops everything; the database stays in `DB_DATA`. To start over with an empty database, run `docker compose down -v` (with the default `DB_DATA`) or delete the `DB_DATA` folder. The [desktop app](desktop/README.md) can back the database up into `backups/` and restore it.

## How it fits together

- `db` is MariaDB 12.3.
- `db-init` runs LSB's `tools/dbtool.py update` and then sets every zone's address to `PUBLIC_IP` ([`docker/lsb/db-init.sh`](docker/lsb/db-init.sh)). The other LSB services wait for it.
- `meshes` copies the navmeshes from LSB's published meshes image into `DATA_DIR` on the first start.
- `connect`, `search`, `world` and `map` are LSB's servers, all from the same image. One map server runs every zone. Each starts through [`docker/lsb/run.sh`](docker/lsb/run.sh), which links the settings in `overrides/settings/` into place, and for `connect` the login certificate in `DATA_DIR/tls/`.
- `ahbot` and `web` are the optional services above, each behind its own Compose profile.
- The PlayOnline profile server (`xi_profile`) and LSB's HTTP API aren't run.
