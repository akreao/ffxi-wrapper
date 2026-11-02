# ffxi-wrapper

Runs a private [LandSandBoat](https://github.com/LandSandBoat/server) (Final Fantasy XI) server and its database in Docker. A desktop app for Linux and Windows that drives it, and client setup with [vana-client](https://github.com/lenonk/vana-client), are planned next.

## Running the server stack

You need Docker with the Compose plugin (Docker Desktop on Windows).

```sh
cp .env.example .env   # then change the passwords
docker compose up -d --build
docker compose logs -f connect world map
```

The first build compiles LandSandBoat from [`akreao/lsb`](https://github.com/akreao/lsb) at the commit pinned in `.env` and takes a while (20 to 60 minutes, depending on the computer). On the first start `db-init` imports LSB's database, which takes a few more minutes; later starts only apply what changed.

| Port | Server |
|------|--------|
| 54231 TCP | login (auth) |
| 54230 TCP | login (data) |
| 54001 TCP | login (view) |
| 54230 UDP | game (map server) |
| 54002 TCP | search |

The client creates its account on the login screen (LSB's `ACCOUNT_CREATION` is on by default).

### Settings in `.env`

- `DATA_DIR` is where the server keeps everything it writes: the database (`db/`), dbtool's record of the database version (`dbtool/`) and the navmeshes (`navmeshes/`, `ximeshes/`). It's `./data` by default; point it at another drive to keep the server's files there. On Linux, the database needs a Linux filesystem (ext4, btrfs, xfs), not exFAT or NTFS.
- `PUBLIC_IP` is the address clients are sent for the map server. Keep `127.0.0.1` when the client runs on this computer; use the computer's LAN or VPN address for players on other computers. It's applied by `db-init` on each start (`docker compose up -d`).
- `BIND_IP`, when set to one of this computer's IPv4 addresses, makes the ports listen on that address only. Docker's port rules go around ufw and firewalld, so this, not the host firewall, keeps the server off other networks.
- `SERVER_NAME` is the server name shown in the client.
- `LSB_REPO` and `LSB_COMMIT` choose the LandSandBoat source to build. Change them, then `docker compose build`.
- `LSB_IMAGE`, when set, uses a ready-made image (such as `ghcr.io/landsandboat/server:latest`, upstream LSB) instead of building.

### Changing LSB's settings

Any LSB setting can be set as an environment variable, `XI_<FILE>_<SETTING>` (for example `XI_MAP_EXP_RATE`), in the `x-lsb` environment in `compose.yaml`. For bigger changes, put a copy of one of LSB's settings files (`settings/default/map.lua` and the rest in the LSB repo) in [`overrides/settings/`](overrides/settings/README.md), keep only the values you change, and restart: `docker compose up -d --force-recreate connect search world map`.

### Stopping and wiping

`docker compose down` stops everything; the database stays in `DATA_DIR`. To start over with an empty database, stop the stack and delete `DATA_DIR/db` and `DATA_DIR/dbtool`.

## How it fits together

- `db` is MariaDB 12.3.
- `db-init` runs LSB's `tools/dbtool.py update` and then sets every zone's address to `PUBLIC_IP` ([`docker/lsb/db-init.sh`](docker/lsb/db-init.sh)). The other LSB services wait for it.
- `meshes` copies the navmeshes from LSB's published meshes image into `DATA_DIR` on the first start.
- `connect`, `search`, `world` and `map` are LSB's servers, all from the same image. One map server runs every zone. Each starts through [`docker/lsb/run.sh`](docker/lsb/run.sh), which links the settings in `overrides/settings/` into place.
- The PlayOnline profile server (`xi_profile`) and LSB's HTTP API aren't run.
