# boswell

## What it is

A daemon that watches one or more git repositories and, when files change, commits and pushes them. It replaces a `systemd` timer that runs `git-sync.sh` once a minute against the project docs in `~/docs`, and possibly against `claude-dotfiles` too.

**The docs are the target.** An earlier version of this document put the dotfiles repository first; that was a misreading. `~/docs` is what a session writes during work and what Tim then reads on GitHub from other devices, so the delay between Claude writing a document and it being readable is the thing worth shortening. A half-finished paragraph published early is harmless there. In `claude-dotfiles` it is not — a half-written `install.sh` is a broken machine — which is why a short debounce suits one repository and not the other, and why they may not want the same treatment at all.

## Why it exists

The timer works, and the reason to replace it is not latency. Watching is nice to have; what the timer cannot do well is fail. A push that cannot land is retried on the next tick with no backoff, and the failure is reported by a separate `OnFailure=` unit that files a GitHub issue. Everything about that path is spread across three units and a shell script.

The one thing a watcher must not lose is that failure reporting. `gitwatch`, the ready-made alternative, runs its push as `eval "$PUSH_CMD"` with no status check: a failed push is not detected, not retried, and does not stop the daemon, so an outage on a quiet repository goes unreported indefinitely. That is the defect boswell exists to not have.

## What mise already does, and where it stops

Since this was written, mise turned out to ship most of this for *configuration files*: `mise dot track <path>` watches a file in place, `[bootstrap.services.mise-history] builtin = "history-watch"` runs the watcher as a user service, and `history.sync = "sync"` with an origin gives two-way sync between machines, plus history, diff, rollback and conflict commands. If the dotfiles repository ever wants a watcher, that is the thing to try first — it is already installed, and `claude-dotfiles` now uses mise for its tool list, its lockfile and its `[dotfiles]` symlinks.

It is the wrong shape for documentation. Its history is a checkpoint stream rather than commits you would read, and its own documentation warns that earlier checkpoints travel to the origin when sync is enabled. The docs repository wants readable subjects, a history worth browsing on GitHub, and nothing intermediate published. So mise covers the dotfiles case and leaves this one open.

One behaviour worth checking before trusting either: mise's documentation says conflicts or unsaved edits can *pause* synchronization. A silent pause is the same defect that rules out `gitwatch` below.

## What already exists

Searched on 2026-09-16. For a general tool that watches a tree and commits what changes, there is one maintained implementation: [gitwatch](https://github.com/gitwatch/gitwatch), 1,736 stars, 486 lines of bash, `inotifywait` plus a debounce. Its push is `eval "$PUSH_CMD"` with no status check, so a failed push is not detected, not retried, and does not stop the daemon — the outage goes unreported on a quiet repository. Everything else in that space has single-digit stars.

The need is clearly real, but it keeps being solved *inside* the application that owns the files. [obsidian-git](https://github.com/Vinzent03/obsidian-git) does precisely this job at 11,991 stars, for an Obsidian vault; Grav has a plugin that does it for a CMS's content folder. `~/docs` was an Obsidian vault on Obsidian Sync until git replaced both, so the closest prior art is a tool this repository's target used to be able to use and no longer can.

That is the gap boswell sits in: auto-committing source code is an anti-pattern, so nobody builds this for developers, and the people who want it are writing content and get it from their editor. An agent writing prose has no editor.

Adjacent, and not substitutes: `git-annex assistant` syncs through annex rather than plain commits, and mise's `history-watch` — the one other maintained watcher — keeps a checkpoint stream for configuration files.

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

Written on the VPS that is being decommissioned; the work continues on the new machine. Nothing here depends on that box.

## Next

1. `cargo init`, dependencies: `notify`, `notify-debouncer-full`, `serde`, `toml`, `clap`, and an HTTP client for the GitHub API.
2. Implement the watch, debounce and commit path; then the push path with its retry and its three outcomes.
3. Tests against temporary repositories with a local bare remote, including a remote made unreachable to exercise the failure path.
4. A release workflow producing a static binary, and the mise entry that installs it.
5. In `claude-dotfiles`: replace `git-sync@.service`, `git-sync@.timer` and `git-sync-failed@.service` with one boswell unit, and move or retire the assertions in `test/assert.sh` that cover them.
