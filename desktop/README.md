# Desktop app

A Tauri app for Linux and Windows that runs the server: it drives `docker compose` on the stack in this repository.

The window follows rag-wrapper's layout: a sidebar of screens (Overview, Server settings, Game client, Play with friends, Backups, Settings), with Start, Stop and a More menu (Restart, Rebuild) in the header on every screen. Overview shows the server's status, the address players connect to, the client, the services and the Activity and log panes. Settings has the server's name and address, the server folder, and a Light/Dark theme choice.

It looks like vana-client's Modern UI (akreao/vana-client, `docs/modern-ui.md`): charcoal panels in its bronze and gold window frame (`ui/modern/frame.svg`, the pack's `window_normal_border`, tiled round the edges), its blue for what's picked and its button, tab, list row and field styles, with Fira Sans for the interface and JetBrains Mono for logs and addresses (`ui/fonts`, SIL OFL, Latin subsets). Dark is that look; Light is a parchment version of it, with the sidebar, header and log panes kept dark.

What it does so far:
- **First run** asks where to keep the server's files (any folder, such as a drive of its own), the address players connect to and the server name. It copies the stack files there and writes `.env` with random database passwords. The database goes in that folder (`DB_DATA=./data/db`) when it's on a Linux filesystem (ext4, btrfs, xfs and the like), and in the `db-data` Docker volume otherwise, which is always the case on Windows. A folder that already has a `.env` keeps it, so an existing setup can be picked. Setup refuses to write new passwords when Docker already has an older `db-data` volume they couldn't open.
- **Start** runs `docker compose up --detach`, which downloads the server image (or builds it when no image is published for the pinned LandSandBoat commit). Before that it re-detects the address players are sent to, unless one is pinned, and checks that no other program holds the server's TCP ports. The output streams into the Activity pane.
- **Restart** recreates `db-init` and the four LSB servers, which applies a new address and settings; the database keeps running.
- **Stop** runs `docker compose down`. The database is kept.
- **Rebuild** builds the server image from source (`docker compose build db-init`). A running server offers a restart onto it.
- **Services** lists each container's state and published ports, refreshed every 3 seconds; clicking one streams its log.
- **Game client** downloads vana-client into `client/` in the server folder (the build of the fork commit pinned in `.env` as `CLIENT_COMMIT`, which the Game client workflow publishes). **Play** works as soon as it's downloaded and opens the game pointed at this server (`127.0.0.1`). Without game data, the client's own first start downloads the game from Square Enix and installs it into `client/game/`, or uses game files already on disk in a folder picked there; the app keeps whichever folder the client saves in `client/config.json`. Installing from Square Enix's installer (`FFXIFullSetup_US.part1.exe` with its other parts beside it) is still offered as an optional step: the app finds it in the server folder or a folder right inside it, or it can be picked. **Update game data** patches the game from Square Enix's servers. A new `CLIENT_COMMIT` shows an update button.
- **Client settings** (Game client > Settings): a set of vana-client's options (window mode and size, frame limit, draw distance, anisotropic filtering, lamp lights, sound, the opening movie, the modern interface and its size, extras such as the minimap, timers, party TP and target distance, and the gamepad: on or off, the sticks' dead zone and inverted camera), edited in place in `client/config.json`, which the app points the client at with `FFXI_CONFIG` (not `~/.config/vana-client/config.json`). Every other key the client keeps there is left alone. Defaults are vana-client's.
- **Game settings**: a set of LandSandBoat's settings (experience, drop, gil, skill-up and crafting rates; level caps, starting gil and inventory, maps, outpost warps and the support job quest level for new characters; fishing and home point teleports). The app keeps them in `overrides/settings/app-settings.json` and writes the ones that differ from LSB's defaults to `overrides/settings/zz_app.lua`, which LSB loads after the other files there, so these win over the same keys in a hand-written `map.lua` or `main.lua`. A running server offers a restart to apply them.
- **Backups**: **Back up now** dumps the database (`mariadb-dump`, gzipped) into `backups/` in the server folder, starting MariaDB for it if the server is stopped. **Stop** backs up first when there's no backup from the last day. The newest 10 made by hand and the newest 10 made automatically are kept. **Restore** stops the game servers, backs up the current database, loads the backup in its place and starts the server again if it was running. LSB's dbtool records the database's version outside the database, so a restored one gets a full `dbtool.py update full` on the next start: LSB's game data is imported again and every migration checked, while accounts and characters are kept.
- **Tray**: closing the window leaves the app in the tray, with Open, Start, Stop, Play and Quit. On a desktop that shows no tray, opening the app again brings the window back.
- **Play with friends** helps players on other networks join over Tailscale, which carries the server's TCP and UDP ports as they are, so nothing is opened on the router. It checks Tailscale on this computer (`tailscale status --json`, or a Tailscale address on an interface when the command isn't on the PATH), offers to send players to the Tailscale address (`PUBLIC_IP`, since LSB hands every client the map server's address), warns when `BIND_IP` keeps the ports off Tailscale, and lists a player's steps: accept a share of this computer, point vana-client at its Tailscale address, and pin the login certificate with a ready-made `vana-client trust-server` command. Without Tailscale, it can look up the network's public (WAN) address with curl (ipify, then icanhazip, then checkip.amazonaws.com) and send players there, looked up again on each start (`PUBLIC_IP_AUTO=wan`; an offline start keeps the last one), with the router's port forwarding spelled out: TCP 54001, 54002, 54230, 54231 and UDP 54230 to this computer's LAN address. Settings offers the same choice; a fixed address, such as a dynamic DNS one's, can be typed there.
- **Settings**: the address players connect to (detect automatically, any address this computer has, `127.0.0.1` or one typed in) and the server name. A running server offers a restart to apply them.

The server keeps running when the app quits. Each app version copies its own `compose.yaml` and `docker/` into the server folder when it starts; `.env`, `overrides/` and `data/` are left alone.

The app keeps its own settings (the server folder) in `config.json` in its config folder (`~/.config/app.ffxiwrapper.desktop/` on Linux, `%APPDATA%\app.ffxiwrapper.desktop\` on Windows).

## Building

It needs Rust, Tauri's CLI (`cargo install tauri-cli --version "^2"`) and, on Linux, WebKitGTK 4.1 and its friends (`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev`).

```sh
cd desktop/src-tauri
cargo test
cargo tauri build --bundles appimage   # or: --bundles nsis on Windows
```

`FFXI_WRAPPER_STACK_DIR=/path/to/clone cargo tauri dev` runs the app on a stack folder as it is, such as a clone of this repository.

The [Desktop app workflow](../.github/workflows/desktop.yml) runs the tests and builds the AppImage and the Windows installer on every pull request that touches the app or the stack, and keeps them as workflow artifacts for 14 days.

## Releases and updates

The app updates itself (Settings > App > Updates, and a bar at the top when a new version is out). It reads `latest.json` from the newest release of this repository and installs a version only if its file is signed with the updater key and the signature names that version. The AppImage replaces itself and restarts; on Windows the new installer runs and opens the app again. A copy run any other way (not as the AppImage, or a build without the key) says so instead.

To publish a version:

1. Set the same `version` in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` (and `Cargo.lock`), and optionally write `release-notes/vX.Y.Z.md`. Merge that.
2. Push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`. The [Release workflow](../.github/workflows/release.yml) builds and signs both, and publishes them with `latest.json` and `.sha256` files.

The updater key is set up once:

```sh
cargo tauri signer generate -w ~/.tauri/ffxi-wrapper-updater.key
```

Put the public key it prints in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`, and the private key file's contents and its password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep the key file: a lost key means every installed copy has to be updated by hand once.
