# Desktop app

A Tauri app for Linux and Windows that runs the server: it drives `docker compose` on the stack in this repository.

What it does so far:
- **First run** asks where to keep the server's files (any folder, such as a drive of its own), the address players connect to and the server name. It copies the stack files there and writes `.env` with random database passwords. The database goes in that folder (`DB_DATA=./data/db`) when it's on a Linux filesystem (ext4, btrfs, xfs and the like), and in the `db-data` Docker volume otherwise, which is always the case on Windows. A folder that already has a `.env` keeps it, so an existing setup can be picked. Setup refuses to write new passwords when Docker already has an older `db-data` volume they couldn't open.
- **Start** runs `docker compose up --detach`, which downloads the server image (or builds it when no image is published for the pinned LandSandBoat commit). Before that it re-detects the address players are sent to, unless one is pinned, and checks that no other program holds the server's TCP ports. The output streams into the Activity pane.
- **Restart** recreates `db-init` and the four LSB servers, which applies a new address and settings; the database keeps running.
- **Stop** runs `docker compose down`. The database is kept.
- **Rebuild** builds the server image from source (`docker compose build db-init`). A running server offers a restart onto it.
- **Services** lists each container's state and published ports, refreshed every 3 seconds; clicking one streams its log.
- **Game client** downloads vana-client into `client/` in the server folder (the build of the fork commit pinned in `.env` as `CLIENT_COMMIT`, which the Game client workflow publishes), then installs the game data into `client/game/` from Square Enix's installer, `FFXIFullSetup_US.part1.exe` with its other parts beside it. It finds the installer in the server folder or a folder right inside it, or it can be picked. **Play** opens the game pointed at this server (`127.0.0.1`); **Update game data** patches the game from Square Enix's servers. The client's own settings are in `client/config.json`. A new `CLIENT_COMMIT` shows an update button.
- **Settings**: the address players connect to (detect automatically, any address this computer has, `127.0.0.1` or one typed in) and the server name. A running server offers a restart to apply them.

The server keeps running when the app closes. Each app version copies its own `compose.yaml` and `docker/` into the server folder when it starts; `.env`, `overrides/` and `data/` are left alone.

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
