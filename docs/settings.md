# Settings

The settings in `.env`, LandSandBoat's own settings, and how players on other networks can join. Back to the [README](../README.md).

## Settings in `.env`

- `DATA_DIR` is the folder where the server keeps its files: dbtool's record of the database version (`dbtool/`) and the navmeshes (`navmeshes/`, `ximeshes/`). The default is `./data`. Point it at another drive to keep the files there.
- `DB_DATA` is where the database lives. The default, `db-data`, is a Docker volume. It works everywhere and is the safe choice on Windows. (To put it on another drive on Windows, use Docker Desktop's Settings, Resources, Disk image location.) On Linux you can use a folder instead, such as `./data/db`, but only on a Linux filesystem (ext4, btrfs or xfs). MariaDB needs Linux file ownership and locking, and exFAT and NTFS don't provide them.
- `PUBLIC_IP` is the address the server gives clients for the map server. Keep `127.0.0.1` when the client runs on this computer. For players on other computers, use this computer's LAN or VPN address. `db-init` applies it on each start (`docker compose up -d`).
- `BIND_IP`: set it to one of this computer's IPv4 addresses and the ports listen on that address only. Docker's port rules go around ufw and firewalld, so this setting, not the host firewall, keeps the server off other networks.
- `SERVER_NAME` is the server name shown in the client. If it is empty, `overrides/settings/main.lua` decides, or LSB's default (Nameless).
- `LSB_REPO` and `LSB_COMMIT` choose the LandSandBoat source. The stack uses `ghcr.io/akreao/ffxi-wrapper/lsb:<LSB_COMMIT>` when GitHub has built it. Otherwise it compiles that commit on this computer.
- `LSB_IMAGE`: if set, the stack uses this image instead, such as `ghcr.io/landsandboat/server:latest` (upstream LSB).

## Players on other networks

The server uses UDP port 54230 for the game itself, so web tunnels such as Cloudflare Tunnel can't carry it. A private network such as Tailscale can. To use one:

1. Set `PUBLIC_IP` to this computer's Tailscale address.
2. Share this computer with each player in Tailscale.
3. Each player points vana-client at that address.

The desktop app's **Play with friends** screen walks you through it.

## Changing LSB's settings

You can set any LSB setting as an environment variable named `XI_<FILE>_<SETTING>` (for example `XI_MAP_EXP_RATE`). Put it in the `x-lsb` environment in `compose.yaml`.

For bigger changes, copy one of LSB's settings files (`settings/default/map.lua` and the others in the LSB repo) into [`overrides/settings/`](../overrides/settings/README.md). Keep only the values you want to change. Then restart:

```sh
docker compose up -d --force-recreate connect search world map
```
