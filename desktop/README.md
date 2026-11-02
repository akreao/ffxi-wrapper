# Desktop app

A Tauri app for Linux and Windows that runs the server: it drives `docker compose` on the stack in this repository.

The window follows rag-wrapper's layout: a sidebar of screens (Overview, Game rules, Game client, Backups, Settings), with Start, Stop and a More menu (Restart, Rebuild) in the header on every screen. Overview shows the server's status, the address players connect to, the client, the services and the Activity and log panes. Settings has the server's name and address, the server folder, and a Light/Dark theme choice.

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
