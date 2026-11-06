#!/usr/bin/env python3
"""Move the wrapper to new LandSandBoat and/or vana-client commits.

    scripts/bump-pins.py --lsb <ref> [--client <ref>]
    scripts/bump-pins.py --client v1.2.0 --dry-run

<ref> is a full or short commit SHA, a tag or a branch name. It's looked up on
GitHub (in LSB_REPO / CLIENT_REPO from .env.example) and written as the full
40-character SHA, which is what the workflows and the app expect. With
--offline, nothing is looked up and only full SHAs are accepted.

Files it changes:
  LSB:    .env.example (LSB_COMMIT), compose.yaml (every LSB_COMMIT default),
          desktop/src-tauri/src/stack.rs (adds the pin to APP_LSB_PINS)
  Client: .env.example (CLIENT_COMMIT)

It prints a diff of what changed. Set GITHUB_TOKEN to avoid GitHub's
unauthenticated rate limit.
"""

import argparse
import difflib
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENV_EXAMPLE = ROOT / ".env.example"
COMPOSE = ROOT / "compose.yaml"
STACK_RS = ROOT / "desktop" / "src-tauri" / "src" / "stack.rs"

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
# What git accepts in a tag or branch name, roughly; keeps the URL safe.
REF = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$")


def fail(msg):
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


def warn(msg):
    print(f"warning: {msg}", file=sys.stderr)


def env_value(text, key):
    found = re.findall(rf"^{key}=(.*)$", text, re.M)
    if not found:
        fail(f"{key} isn't set in .env.example")
    return found[-1].strip()


def github_slug(repo_url):
    m = re.match(r"^https://github\.com/([^/]+/[^/]+?)(?:\.git)?/?$", repo_url)
    if not m:
        fail(f"{repo_url} isn't a GitHub repository URL")
    return m.group(1)


def github(path):
    req = urllib.request.Request(
        f"https://api.github.com/{path}",
        headers={"Accept": "application/vnd.github+json", "User-Agent": "ffxi-wrapper-bump-pins"},
    )
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.load(resp)
    except urllib.error.HTTPError as e:
        if e.code in (404, 422):
            return None
        fail(f"GitHub answered {e.code} for {path}: {e.read().decode(errors='replace')[:200]}")
    except urllib.error.URLError as e:
        fail(f"couldn't reach GitHub ({e.reason}); use --offline with a full SHA")


def resolve(name, ref, repo_url, offline):
    """Turns ref into a full SHA, checking it exists in the repository."""
    ref = ref.strip()
    if offline:
        if not FULL_SHA.match(ref.lower()):
            fail(f"--offline needs the full 40-character {name} commit SHA, got '{ref}'")
        return ref.lower()
    if not REF.match(ref) or ".." in ref:
        fail(f"'{ref}' doesn't look like a commit SHA, tag or branch")
    slug = github_slug(repo_url)
    info = github(f"repos/{slug}/commits/{ref}")
    if not info:
        fail(f"{slug} has no commit, tag or branch '{ref}'")
    sha = info["sha"]
    title = info["commit"]["message"].splitlines()[0]
    print(f"{name}: {ref} -> {sha} ({title})")
    if not re.match(r"^[0-9a-fA-F]{4,40}$", ref) and github(f"repos/{slug}/branches/{ref}"):
        warn(f"'{ref}' is a branch; pinning the commit it points at right now")

    # A pin has to stay buildable, so it should be on the default branch,
    # not only on a branch that may be reset or deleted.
    default = (github(f"repos/{slug}") or {}).get("default_branch")
    if default:
        cmp = github(f"repos/{slug}/compare/{default}...{sha}")
        if cmp and cmp.get("status") not in ("identical", "behind"):
            warn(f"{sha[:12]} isn't on {slug}'s default branch '{default}'; "
                 "make sure the branch it's on won't be reset")
    return sha


def bump_lsb(files, new):
    env = files[ENV_EXAMPLE]
    old = env_value(env, "LSB_COMMIT")
    files[ENV_EXAMPLE] = re.sub(r"^LSB_COMMIT=.*$", f"LSB_COMMIT={new}", env, count=0, flags=re.M)

    compose, n = re.subn(r"(LSB_COMMIT:-)[0-9a-fA-F]+", rf"\g<1>{new}", files[COMPOSE])
    if n == 0:
        fail("found no ${LSB_COMMIT:-<sha>} default in compose.yaml; update the script")
    files[COMPOSE] = compose

    # The app empties a .env LSB_COMMIT holding any pin it ever shipped, so
    # the old pin must stay listed and the new one added (a test checks).
    rs = files[STACK_RS]
    m = re.search(r"(const APP_LSB_PINS: \[&str; )(\d+)(\] = \[\n)(.*?)(^\];)", rs, re.S | re.M)
    if not m:
        fail("couldn't find APP_LSB_PINS in stack.rs; update the script")
    pins = re.findall(r'"([0-9a-f]{40})"', m.group(4))
    for pin in (old, new):
        if FULL_SHA.match(pin) and pin not in pins:
            pins.append(pin)
    body = "".join(f'    "{p}",\n' for p in pins)
    files[STACK_RS] = (rs[: m.start()] + m.group(1) + str(len(pins)) + m.group(3) + body
                       + m.group(5) + rs[m.end():])
    return old


def bump_client(files, new):
    env = files[ENV_EXAMPLE]
    old = env_value(env, "CLIENT_COMMIT")
    files[ENV_EXAMPLE] = re.sub(r"^CLIENT_COMMIT=.*$", f"CLIENT_COMMIT={new}", env, flags=re.M)
    return old


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--lsb", metavar="REF", help="LandSandBoat commit SHA, tag or branch")
    p.add_argument("--client", metavar="REF", help="vana-client commit SHA, tag or branch")
    p.add_argument("--dry-run", action="store_true", help="show the diff without writing")
    p.add_argument("--offline", action="store_true", help="don't look anything up; full SHAs only")
    args = p.parse_args()
    if not args.lsb and not args.client:
        p.error("give --lsb and/or --client")

    paths = [ENV_EXAMPLE, COMPOSE, STACK_RS]
    files = {path: path.read_text(encoding="utf-8") for path in paths}
    before = dict(files)
    env = files[ENV_EXAMPLE]

    if args.lsb:
        sha = resolve("LSB", args.lsb, env_value(env, "LSB_REPO"), args.offline)
        old = bump_lsb(files, sha)
        print(f"LSB_COMMIT: {old} -> {sha}" if old != sha else f"LSB_COMMIT is already {sha}")
    if args.client:
        sha = resolve("vana-client", args.client, env_value(env, "CLIENT_REPO"), args.offline)
        old = bump_client(files, sha)
        print(f"CLIENT_COMMIT: {old} -> {sha}" if old != sha else f"CLIENT_COMMIT is already {sha}")

    changed = [path for path in paths if files[path] != before[path]]
    if not changed:
        print("Nothing to change.")
        return
    for path in changed:
        rel = path.relative_to(ROOT).as_posix()
        sys.stdout.writelines(difflib.unified_diff(
            before[path].splitlines(keepends=True), files[path].splitlines(keepends=True),
            f"a/{rel}", f"b/{rel}"))
        if not args.dry_run:
            path.write_text(files[path], encoding="utf-8", newline="")
    if args.dry_run:
        print("\nDry run: nothing written.")
    else:
        print(f"\nUpdated {', '.join(p.relative_to(ROOT).as_posix() for p in changed)}.")


if __name__ == "__main__":
    main()
