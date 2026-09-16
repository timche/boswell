# boswell

A daemon that watches a git repository and commits what changes, so the work of an agent editing files publishes itself.

Named for James Boswell, who followed Samuel Johnson around writing down everything he said and published it.

## Install

```sh
mise use -g ubi:timche/boswell
```

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
pull = true         # on a rejected push, pull --rebase --autostash then push again
recheck = "10m"     # while commits remain unpushed, retry the push this often even with no file changes
```

`[retry]` is global; `debounce`, `remote`, `pull` and `recheck` are per repository. A config listing no repositories, or a path that is not a git work tree, is rejected at startup.

## Run

`boswell` watches every configured repository until stopped. `boswell once` runs a single sync pass over all of them and exits non-zero if any ended in failure, which is what a backstop timer would call. Both take `--config PATH`. Logging goes to stderr at info level; `RUST_LOG` overrides it.

A single pass waits out the whole retry ladder against an unreachable remote — about ten minutes at the defaults — so a timer unit calling `boswell once` wants a `TimeoutStartSec` longer than that, or a shorter `[retry]`.

## When a push cannot land

An unreachable remote is retried with exponential backoff; a push that needs a human — a non-fast-forward with `pull = false`, or a rebase that conflicts — is not retried at all. Either way boswell opens an issue titled `Auto-sync failed` against the repository's own origin, containing the error, the commits that are stuck locally and anything still uncommitted.

While that issue is open no further one is filed, so fixing the push and then **closing the issue** is what re-arms reporting. The token comes from `GH_TOKEN`, then `GITHUB_TOKEN`, then `gh auth token`, and is only looked up when an issue has to be filed.
