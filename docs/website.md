# Website

Back to the [README](../README.md).

The optional `web` service is a website for the server.

**What it has**

- A home page.
- **My account** for every game account: its characters (read-only), a password change, and, while the server has player bots on, a per-character switch that hides the AI adventurers (the same as `!bots off` in game; it counts from the character's next login).
- An admin panel for accounts with the Moderator or Owner website role:
  - a dashboard (who is online, busiest zones, service status, recent actions);
  - searchable Accounts and Characters, with a game-masters-only filter (`/admin/characters?gm=1`);
  - Bans;
  - an Admin log.

**What staff can do**

- Staff ban and unban accounts. A ban sets `accounts.status` and keeps its end date and reason in `accounts_banned`. The site lifts timed bans when they run out. A ban doesn't remove anyone who is already in the game.
- Only owners reset passwords and two-step sign-in, change in-game GM levels and set website roles.
- Nobody can act on an account whose website role is the same as theirs or higher.
- Moderators leave accounts that have an in-game GM character to owners.
- Every change needs a reason where it matters, and is written to the log.

**Turning it on**

- It is off unless `.env` has `COMPOSE_PROFILES=web` (comma-separated with `ahbot` if you use both).
- It listens on this computer only: <http://127.0.0.1:54080>.
- `WEB_BIND_IP` in `.env` opens it to a LAN or Tailscale address of this computer. `WEB_PORT` changes the port.
- It also always answers on <http://127.0.0.1:54081> (`WEB_LOCAL_PORT`), this computer only. The desktop app's owner link opens there.
- On its own the site speaks plain HTTP on that port. To put it on the internet, use the `public` profile below, never this port.

## How it works

- **Signing in.** Every game account signs in with its login and password.
  - Passwords stay LSB's own bcrypt hashes. An account with an old-style hash has to log in to the game once first. Two-step sign-in is honoured.
  - **Website roles and GM levels are separate.** A website role (Player, Moderator or Owner) belongs to an account and is kept in the site's own database. An owner sets it on the account's page, and only the owner link can take Owner away from someone. A GM level belongs to a character (LSB's `chars.gmlevel`) and only controls in-game GM commands. So giving a character GM 3 for `!goto`, or using `!promote` in game, gives no website access.
  - When a site that already ran an older version first starts with this, it copies the roles it used to work out from GM levels, once, so nobody loses access. It uses the highest level among an account's characters: 5 is Owner, 3 and 4 are Moderator. A new or wiped site copies nothing. Open the admin panel with the desktop app's owner link and set roles there.
  - The role is read again on every request, so a ban or role change takes effect at once.
  - Failed sign-ins are limited per address and login, and every change needs a CSRF token.
- **Desktop app.** The app's **Website** screen turns the site on and off, picks who can reach it and opens the admin panel signed in as the owner. It generates `ADMIN_SECRET` when there is none.
- **The owner link.** `ADMIN_SECRET` (64 hex characters, made with `openssl rand -hex 32`) signs a link that works for one minute and one use: `/auth/owner?t=...`. It signs the owner in without a password. An optional `next` picks the admin panel page to land on. Only plain `/admin` paths with no `%` escapes work; anything else lands on `/admin`. If `ADMIN_SECRET` is empty, the link is off.
- **Its database user.** The site connects as `xiweb`, which `db-init` creates from `DB_ROOT_PASSWORD`. It has its own password, `WEB_DB_PASSWORD`. The web container never gets `xiadmin`'s password. `xiweb` can do only this:
  - `SELECT` on the tables its pages read;
  - `UPDATE` on `accounts.password`;
  - `INSERT` on the seven `accounts` columns that LSB's own account creation fills.

  Later phases add the grants they need to `docker/lsb/db-init.sh`.
- **Sign-up.** Visitors can create a game account on the site. It is made exactly as LSB's login server makes one (`LOGIN_CREATE`).
  - Sign-up works only while the owner has it open. It is closed until switched on (**Invite codes** in the admin panel), and closed if the site can't read its own settings.
  - LSB's `login.ACCOUNT_CREATION` isn't read. It covers only accounts made from the game's login screen. To make the site the only way to get an account, switch it off (in a settings file or the desktop app's Game settings).
  - A login is 3-16 letters or numbers. A password is 8-32 characters.
  - By default a sign-up also needs an invite code. Only the owner makes them (**Invite codes** in the admin panel). Each code is shown once. You can switch the requirement off there.
  - Limits: 3 sign-ups an hour per address, and 30 accounts a day for the whole server.
  - The new account is signed in and taken to **My account**, which shows how to connect.
- **Audit logs.** Staff read LSB's own logs in the admin panel: chat, GM commands, trades, bazaar sales and delivery box sends. They can filter by log, chat channel, character or account, dates and text. Newest comes first. Tells are shown to the owner only. The Rules page tells players that the server may log chat.
  - LSB writes these logs only for what the desktop app's Game settings switch on (Chat and moderation: *Log chat to the database* plus each channel, *Log GM commands*, trades, bazaar, delivery box). All are off by default.
  - **Retention:** LSB never deletes from `audit_chat`, `audit_gm`, `audit_trade`, `audit_bazaar` or `audit_dbox`, and the site only reads them (`SELECT`). They grow for as long as logging is on.
  - The site shows the last 90 days, at most 40 pages per search. It runs at most two searches at once and stops a search after 5 seconds.
  - Every search goes in the Admin log, where only the owner sees it. The entry records which log and filters were used and whether the search ran, but not what it found.
  - To trim old rows, delete them as `xiadmin`, for example `DELETE FROM audit_chat WHERE datetime < NOW() - INTERVAL 180 DAY;`.
- **What it keeps.** The site's own SQLite file (sessions and the admin action log) is `DATA_DIR/web/web.db`. Nothing is added to the game database. Backups from the desktop app don't include this file yet.
- **The image.** [`docker/web/`](../docker/web/Dockerfile) builds the Rust service in [`web/`](../web/) with the shared crate [`core/`](../core/). The [`Website image`](../.github/workflows/web-image.yml) workflow publishes `ghcr.io/akreao/ffxi-wrapper/web:<version>` and tags the commit `web-v<version>`. The version is the one in `web/Cargo.toml`. `compose.yaml` names it twice (the image, and the `#web-v<version>` build fallback). A pull request that changes `web/`, `core/` or `docker/web/` has to bump all three. The site's pages share `desktop/ui/tokens.css` (fonts, colours, frames and controls) with the desktop app.

## Website on the internet

The `public` profile adds a [Caddy](https://caddyserver.com) container (`proxy`). It:

- answers on ports 80 and 443;
- gets and renews a Let's Encrypt certificate by itself;
- sends plain-HTTP visitors to HTTPS;
- passes everything else to `web`.

TLS ends on this computer, so nobody else sees players' passwords. Caddy's settings are in [`docker/caddy/Caddyfile`](../docker/caddy/Caddyfile). To set it up:

1. Pick a name, such as `ffxi.example.com`, and make a DNS **A record** for it that points at this computer's internet address. With Cloudflare, keep the record **DNS only** (grey cloud). Its proxy would decrypt the traffic again, and it can't carry the other ports anyway. If your home address changes, step 4 keeps the record current.
2. On the router, forward **TCP 80 and 443** to this computer. Port 80 is for the certificate check and the redirect. The game's own ports are not part of this.
3. In `.env`, set `COMPOSE_PROFILES=web,public` (plus `ahbot` if you use it) and `WEB_DOMAIN=ffxi.example.com`. `WEB_PUBLIC_URL` is worked out from the domain; set it only to override. Then run `docker compose up -d`. The first visit takes a few seconds while the certificate is issued. `docker compose logs proxy` shows how it went. Certificates and the account key are kept in `DATA_DIR/caddy/`, so keep that folder private.
4. Optional, if your address changes: add `ddns` to `COMPOSE_PROFILES` and put a Cloudflare API token in `.env` as `CLOUDFLARE_API_TOKEN`. Create the token with only **Zone > DNS > Edit** on the domain's zone. The `ddns` service ([favonia/cloudflare-ddns](https://github.com/favonia/cloudflare-ddns)) then keeps the A record for `WEB_DOMAIN` on this computer's address, unproxied. The token is read from `.env` only. Don't paste it anywhere else.

How the site behaves behind the proxy:

- It trusts the proxy's `X-Forwarded-For` only when the connection comes from the `proxy` container itself (found by name, re-checked every 15 seconds). It then uses the last entry, which is the address Caddy saw. A visitor can't pick the address that rate limits, sessions and the admin log record.
- Caddy answers nothing for any name other than `WEB_DOMAIN`, and the site itself refuses one that gets through (421).
- Over HTTPS the site sends HSTS and `Secure` cookies.
- While `WEB_PUBLIC_URL` is set, signing in over plain HTTP is refused unless it comes from this computer, a private network address or Tailscale. The direct port `54080` keeps working for the owner and the LAN, but not for the internet.
- Without a domain, Tailscale stays the way in.

## Adding to the site

- **A route** is a handler plus one `.route(...)` line in the module for its audience under [`web/src/routes/`](../web/src/routes/) (`public`, `session`, `me`, `admin`).
- **A page** is a file in [`web/ui/`](../web/ui/), plus a line in `routes/pages.rs` and a function in `ui/app.js`.
- **A query** is a method on the `Game` trait in `web/src/game.rs`, with a fake in `web/tests/common/`.

Run the tests with `cargo test` in `web/` and in `core/`.
