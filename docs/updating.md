# Updating the server and client

How the server image and the game client are built, and how to move to a newer version of either. Back to the [README](../README.md).

## Server image

The [`LSB server image`](../.github/workflows/lsb-image.yml) workflow compiles LandSandBoat from `LSB_REPO` at the `LSB_COMMIT` pinned in `.env.example`. It publishes the result as `ghcr.io/akreao/ffxi-wrapper/lsb:<commit>` (and `:latest`) for linux/amd64. It runs when that pin changes on `main`, and by hand from the Actions tab, where it can also build any other commit. A commit that is already published isn't built again.

To move the server to a newer LSB commit:

1. Change `LSB_COMMIT` in both `.env.example` and `compose.yaml` (its default).
2. Add the new commit to `APP_LSB_PINS` in `desktop/src-tauri/src/stack.rs`. A test fails until you do.
3. Merge.

`scripts/bump-pins.py --lsb <commit>` does all three (see Bumping the pins below).

The app leaves `LSB_COMMIT` empty in its `.env`, so it always uses the pin it shipped with. Once the workflow has published the image, `docker compose up -d` pulls it and recreates the servers.

To compile on this computer anyway, run `docker compose build db-init` (or `docker compose up -d --build`).

## Game client

The [`Game client`](../.github/workflows/client.yml) workflow builds vana-client from `CLIENT_REPO` at the `CLIENT_COMMIT` pinned in `.env.example`. It builds for Linux (in the Steam Runtime SDK, so it runs on any current distribution) and for Windows. It publishes both as a release of this repository, tagged `client-<first 12 characters of the commit>`. Each file has its SHA-256 next to it (`<file>.sha256`).

The desktop app downloads the build for its platform from that release. It installs the build only if it matches the checksum. Like the server image, the workflow runs when the pin changes on `main`, and by hand from the Actions tab. A commit that is already published isn't built again unless you ask.

To move to a newer client, push the change to the fork, put its full commit SHA in `CLIENT_COMMIT` in `.env.example`, and merge. Once the release is published, the app offers the update.

## Bumping the pins

`scripts/bump-pins.py` updates every file that pins either commit. Give it a full or short SHA, a tag or a branch name. It looks the commit up on GitHub (in `LSB_REPO` or `CLIENT_REPO`), writes the full SHA and prints a diff:

```sh
scripts/bump-pins.py --lsb 1a2b3c4d --client v1.2.0
scripts/bump-pins.py --client <sha> --dry-run   # show the diff, write nothing
```

- For LSB it changes `.env.example` and all of `compose.yaml`'s defaults, and adds the new pin to `APP_LSB_PINS`.
- For the client it changes `.env.example`.
- It warns when the commit isn't on the repository's default branch, because a pin there could be lost if its branch is reset.
- `--offline` skips the lookup and takes full SHAs only.
- Set `GITHUB_TOKEN` if you hit GitHub's rate limit.
- It needs Python 3.10 or later.

The [`Checks`](../.github/workflows/checks.yml) workflow runs on every pull request. It:

- tests `bump-pins.py` (`python3 -m unittest discover -s scripts`);
- fails if `.env.example`, `compose.yaml` and `APP_LSB_PINS` disagree on a pin;
- checks that `compose.yaml` parses;
- runs ShellCheck on the stack's shell scripts;
- checks that the UI's JavaScript parses;
- runs actionlint on the workflows.

## The login certificate

vana-client remembers each server's login certificate and refuses a different one later.

`connect` makes a certificate on its first start and keeps it in `DATA_DIR/tls/` (`login.cert` and `login.key`). Recreating the container or moving to a new server image keeps the same certificate.

- **Desktop app:** it pins the certificate in its own client before each start, so that client never has to trust it on first use.
- **Players on other computers:** they can pin it with `vana-client trust-server SERVER --port 54231 --fingerprint ...`. Use the key fingerprint the desktop app shows under Play with friends, or work it out with this command:

  ```sh
  openssl x509 -in DATA_DIR/tls/login.cert -noout -pubkey | openssl pkey -pubin -outform DER | openssl dgst -sha256
  ```

  This hashes the key, not the whole certificate, so don't use `-fingerprint`.
- **Starting over:** deleting `DATA_DIR/tls/` makes a new certificate on the next start. Every player's client then has to trust it again.
