# boswell

## What it is

A daemon that watches one or more git repositories and, when files change, commits and pushes them. It replaces a `systemd` timer that ran `git-sync.sh` once a minute against `claude-dotfiles` and the project docs.

## Why it exists

The timer works, and the reason to replace it is not latency. Watching is nice to have; what the timer cannot do well is fail. A push that cannot land is retried on the next tick with no backoff, and the failure is reported by a separate `OnFailure=` unit that files a GitHub issue. Everything about that path is spread across three units and a shell script.

The one thing a watcher must not lose is that failure reporting. `gitwatch`, the ready-made alternative, runs its push as `eval "$PUSH_CMD"` with no status check: a failed push is not detected, not retried, and does not stop the daemon, so an outage on a quiet repository goes unreported indefinitely. That is the defect boswell exists to not have.

## Decisions

- **Rust.** `notify` plus `notify-debouncer-full` gives recursive watching and event coalescing as library behaviour, including directories created after start — the fiddly half of the job. Go's `fsnotify` is non-recursive on Linux and would mean hand-rolling both. No async runtime: the debouncer hands over a channel and a blocking loop reads it.
- **Shell out to `git`.** Identical semantics to what a person would type, and `git` is on every machine this runs on. No `gix`, no `git2`.
- **One binary, several repositories.** A small TOML config lists them; one process watches all of them rather than one unit per repository, which is what the templated timer needed.
- **Distributed as a GitHub release, installed by mise** (`ubi:timche/boswell`). Not published to crates.io: nobody depends on this as a library, `cargo install` would mean a Rust toolchain and a source build on the target box, and crates.io versions can only be yanked, never deleted. The crate name is taken by an unrelated retry library anyway; the binary name is what matters and it is free.
- **Public repository.**
- **Rust pinned per project** in `mise.toml`, which is also the first project to exercise the rule that a tool only one project needs is declared by that project.

## Shape

- Config lists repositories, each with a debounce and a remote.
- A change to a watched tree starts the debounce; further changes restart it, so a burst of writes lands as one commit.
- Commit message generation is the one thing worth carrying over from `git-sync.sh`: a sentence-case subject, and the changed file list joined with a comma and a space. `claude-dotfiles`' `test/assert.sh` asserts both, so whatever boswell writes has to keep satisfying them or those assertions move here.
- Push with bounded retry and exponential backoff. The three outcomes are distinct and handled separately: nothing to push, the push was rejected (needs a pull first), the remote was unreachable.
- Only when retries are exhausted does it open a GitHub issue, through the API with the token from `gh auth token` rather than by shelling out to `gh` — the failure path should not depend on another binary. One open issue suppresses further ones, which is what the current `git-sync-failed.sh` does, and closing it is what re-arms reporting.

## Not yet decided

- Whether a long-interval backstop timer survives alongside it. In-process retry covers a failed push, but nothing covers the daemon being dead. A timer that only pushes, wired to the existing issue filer, is cheap insurance; the alternative is a watchdog on the unit.
- Whether boswell also handles the initial `git pull --rebase` a second machine would need. The current setup never pulls, because one machine writes.
- Default debounce. The timer's minute was deliberate: a short window means a half-finished edit can be upstream before the next write finishes it, which is the known failure mode of the current design. A watcher with a two second debounce makes that more likely, not less.

## State

The repository exists and is empty apart from `mise.toml` pinning Rust 1.98, a `.gitignore` for `/target`, and these documents. No `cargo init` yet, no code.

## Next

1. `cargo init`, dependencies: `notify`, `notify-debouncer-full`, `serde`, `toml`, `clap`, and an HTTP client for the GitHub API.
2. Implement the watch, debounce and commit path; then the push path with its retry and its three outcomes.
3. Tests against temporary repositories with a local bare remote, including a remote made unreachable to exercise the failure path.
4. A release workflow producing a static binary, and the mise entry that installs it.
5. In `claude-dotfiles`: replace `git-sync@.service`, `git-sync@.timer` and `git-sync-failed@.service` with one boswell unit, and move or retire the assertions in `test/assert.sh` that cover them.
