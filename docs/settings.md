# Settings

Everything you can change in `.env`, in LandSandBoat's own settings, and how to reach the server from other networks. Back to the [README](../README.md).

## Settings in `.env`

- `DATA_DIR` is where the server keeps its files: dbtool's record of the database version (`dbtool/`) and the navmeshes (`navmeshes/`, `ximeshes/`). It's `./data` by default; point it at another drive to keep them there.
- `DB_DATA` is where the database lives. The default, `db-data`, is a Docker volume, which works everywhere and is the reliable choice on Windows (Docker Desktop's Settings, Resources, Disk image location moves it to another drive). On Linux it can be a folder instead, such as `./data/db`, but only on a Linux filesystem (ext4, btrfs, xfs): MariaDB needs Linux file ownership and locking, which exFAT and NTFS don't give it.
- `PUBLIC_IP` is the address clients are sent for the map server. Keep `127.0.0.1` when the client runs on this computer; use the computer's LAN or VPN address for players on other computers. It's applied by `db-init` on each start (`docker compose up -d`).
- `BIND_IP`, when set to one of this computer's IPv4 addresses, makes the ports listen on that address only. Docker's port rules go around ufw and firewalld, so this, not the host firewall, keeps the server off other networks.
- `SERVER_NAME` is the server name shown in the client. Left empty, `overrides/settings/main.lua` (or LSB's default, Nameless) decides.
- `LSB_REPO` and `LSB_COMMIT` choose the LandSandBoat source. The stack uses `ghcr.io/akreao/ffxi-wrapper/lsb:<LSB_COMMIT>` when GitHub has built it, and compiles that commit here otherwise.
- `LSB_IMAGE`, when set, uses another image instead (such as `ghcr.io/landsandboat/server:latest`, upstream LSB).

## Players on other networks

The server's ports include UDP 54230 (the game itself), so web tunnels such as Cloudflare Tunnel can't carry it. A private network such as Tailscale can: set `PUBLIC_IP` to this computer's Tailscale address, share the computer with each player in Tailscale, and players point vana-client at that address. The desktop app's **Play with friends** screen walks through it.

## Changing LSB's settings

Any LSB setting can be set as an environment variable, `XI_<FILE>_<SETTING>` (for example `XI_MAP_EXP_RATE`), in the `x-lsb` environment in `compose.yaml`. For bigger changes, put a copy of one of LSB's settings files (`settings/default/map.lua` and the rest in the LSB repo) in [`overrides/settings/`](../overrides/settings/README.md), keep only the values you change, and restart: `docker compose up -d --force-recreate connect search world map`.
