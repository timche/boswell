# boswell

A daemon that watches a git repository and commits what changes, so the work of an agent editing files publishes itself.

Named for James Boswell, who followed Samuel Johnson around writing down everything he said and published it.

## Install

```sh
brew install timche/tap/boswell
```

The formula in [timche/homebrew-tap](https://github.com/timche/homebrew-tap) installs the release binary and is updated by every release. mise installs the same binaries straight from the release, for a machine that would rather not have Homebrew:

```sh
mise use -g github:timche/boswell
```

Every release carries a static x86-64 Linux binary and an Apple Silicon macOS one, and both routes pick between those two. Anywhere else — another Linux architecture, an Intel Mac, a BSD — means building from source against the toolchain `mise.toml` pins: `cargo build --release`.

## Configure

`$XDG_CONFIG_HOME/boswell/config.toml`, or `~/.config/boswell/config.toml`. Everything but `[[repo]].path` has a default, and every value below is shown at its default.

```toml
[github]
api_url = "https://api.github.com"

[retry]
attempts = 6        # push attempts before an issue is filed
base = "5s"         # first backoff delay, doubling each attempt
max = "5m"          # cap on a single delay

[[repo]]
path = "~/docs"
debounce = "5s"     # quiet time after the last change before a sync
remote = "origin"
pull = true         # rebase onto the remote before pushing, and on the periodic fetch
recheck = "10m"     # while commits remain unpushed, retry the push this often even with no file changes
fetch_interval = "1m"  # fetch and rebase onto the remote this often, even with no local changes; "0s" disables
```

`[retry]` is global; `debounce`, `remote`, `pull`, `recheck` and `fetch_interval` are per repository. A config listing no repositories, or a path that is not a git work tree, is rejected at startup.

With `pull` on, every sync pass fetches first and rebases onto the remote before it pushes, so what another machine wrote arrives before what this one wrote goes up; local changes are committed first, so the rebase runs over a checkpoint rather than over half-written work. The daemon also runs that pass on `fetch_interval` with nothing changed locally, which is how a second machine's pushes reach this one. After a rebase conflict it stops pulling that repository on the interval until the `Auto-sync failed` issue is closed, so it does not re-run a rebase that can only conflict again.

## Run

`boswell` watches every configured repository until stopped. `boswell once` runs a single sync pass over all of them and exits non-zero if any ended in failure, which is what a backstop timer would call. Both take `--config PATH`. Logging goes to stderr at info level; `RUST_LOG` overrides it.

A single pass waits out the whole retry ladder against an unreachable remote — a few minutes at the defaults, plus the time each push spends timing out — so a timer unit calling `boswell once` wants a `TimeoutStartSec` longer than that, or a shorter `[retry]`.

### As a service

boswell stays in the foreground and exits non-zero when a watcher dies rather than running blind, so whatever starts it has to be what restarts it: a systemd user unit with `Restart=on-failure` on Linux, a launchd LaunchAgent on macOS. Give either one an absolute path rather than a bare `boswell`, because neither starts from a shell that has a PATH: `/opt/homebrew/bin/boswell` for a Homebrew install, or mise's shim for a mise one, which resolves the pinned version without an activated shell.

The agent goes in `~/Library/LaunchAgents/io.github.timche.boswell.plist`; launchd does not expand `~` inside it, so the paths are spelled out.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>io.github.timche.boswell</string>
  <key>ProgramArguments</key>
  <array>
    <string>/opt/homebrew/bin/boswell</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <!-- Restart on a non-zero exit, and only on one. -->
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardErrorPath</key>
  <string>/Users/you/Library/Logs/boswell.log</string>
</dict>
</plist>
```

`launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/io.github.timche.boswell.plist` loads it, `bootout` in place of `bootstrap` unloads it, and the log is where the stderr above lands.

## When a push cannot land

An unreachable remote is retried with exponential backoff; a push that needs a human — a non-fast-forward with `pull = false`, or a rebase that conflicts — is not retried at all. Either way boswell opens an issue titled `Auto-sync failed` against the repository's own origin, containing the error, the commits that are stuck locally and anything still uncommitted.

While that issue is open no further one is filed, so fixing the push and then **closing the issue** is what re-arms reporting. The token comes from `GH_TOKEN`, then `GITHUB_TOKEN`, then `gh auth token`, and is only looked up when an issue has to be filed.
