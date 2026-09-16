# boswell

## What it is

A daemon that watches one or more git repositories and, when files change, commits and pushes them. It exists for files that are written by something other than a person at an editor, most often an agent writing documentation during a session, and read somewhere else, most often on GitHub from another device. The delay between the write and the read is the thing worth shortening.

Not every repository wants the same treatment. A half-finished paragraph published early is harmless in a notes repository. A half-written install script in a configuration repository is a broken machine. The per-repository debounce is what separates the two: seconds for prose, a minute or more for anything that is executed.

## Why it exists

The obvious predecessor is a cron or systemd timer running a shell script every minute, and the reason to replace it is not latency. Watching is nice to have; what a timer cannot do well is fail. A push that cannot land is retried on the next tick with no backoff, and reporting the failure means a second unit and a second script wired to the first one's `OnFailure=`.

The one thing a watcher must not lose is that failure reporting. `gitwatch`, the ready-made alternative, runs its push as `eval "$PUSH_CMD"` with no status check: a failed push is not detected, not retried, and does not stop the daemon, so an outage on a quiet repository goes unreported indefinitely. That is the defect boswell exists to not have.

## What already exists

Searched on 2026-09-16. For a general tool that watches a tree and commits what changes, there is one maintained implementation: [gitwatch](https://github.com/gitwatch/gitwatch), 1,736 stars, 486 lines of bash, `inotifywait` plus a debounce, with the push defect above. Everything else in that space has single-digit stars.

The need is clearly real, but it keeps being solved *inside* the application that owns the files. [obsidian-git](https://github.com/Vinzent03/obsidian-git) does precisely this job at 11,991 stars, for an Obsidian vault; Grav has a plugin that does it for a CMS's content folder. That is the gap boswell sits in: auto-committing source code is an anti-pattern, so nobody builds this for developers, and the people who want it are writing content and get it from their editor. An agent writing prose has no editor.

Adjacent, and not substitutes: `git-annex assistant` syncs through annex rather than plain commits. mise's `history-watch` watches configuration files and keeps their history as git commits, but the subjects are trigger labels such as `edit`, every intermediate checkpoint travels to the origin when sync is on, and a conflict pauses sync with a desktop notification, which on a headless machine is nothing at all. It is the right tool for dotfiles that stay on one person's machines and the wrong shape for a history anyone will read.

## Decisions

- **Rust.** `notify` plus `notify-debouncer-full` gives recursive watching and event coalescing as library behaviour, including directories created after start, which is the fiddly half of the job. Go's `fsnotify` is non-recursive on Linux and would mean hand-rolling both. No async runtime: the debouncer hands over a channel and a blocking loop reads it.
- **Shell out to `git`.** Identical semantics to what a person would type, and `git` is on every machine this runs on. No `gix`, no `git2`.
- **One binary, several repositories.** A small TOML config lists them; one process watches all of them rather than one unit per repository.
- **Distributed as a GitHub release, installed by mise** (`ubi:timche/boswell`) or by downloading the static binary. Not published to crates.io: nobody depends on this as a library, `cargo install` would mean a Rust toolchain and a source build on the target box, and crates.io versions can only be yanked, never deleted. The crate name is taken by an unrelated retry library anyway; the binary name is what matters and it is free.
- **Public repository.**
- **Rust pinned in `mise.toml`**, with rustfmt and clippy declared as components so CI and a fresh checkout get the same toolchain.

## Shape

- Config lists repositories, each with a debounce and a remote.
- A change to a watched tree starts the debounce; further changes restart it, so a burst of writes lands as one commit.
- `notify` cannot exclude a subtree from a recursive watch, so `.git` is watched like everything else and its events are discarded after the fact. The cost is a few hundred inotify watches per repository that exist only to be ignored.
- The commit subject is `Update` followed by the changed paths joined with a comma and a space, and after three paths a count of the rest. Sentence case, so it reads like a hand-written subject in the log.
- Push with bounded retry and exponential backoff. The three outcomes are distinct and handled separately: nothing to push, the push was rejected (needs a pull first), the remote was unreachable.
- Only when retries are exhausted, or a rebase conflicts, does it open a GitHub issue, through the API with a token from the environment or `gh auth token` rather than by shelling out to `gh`, so the failure path does not depend on another binary. One open issue suppresses further ones, and closing it is what re-arms reporting.
- A repository thread that dies takes the whole process down with a non-zero exit. A supervisor restarting a crashed daemon is a solved problem; a daemon that looks alive and watches nothing is the failure this tool exists to avoid.

## Defaults, and why

- **Debounce: five seconds.** A minute, the interval a timer would use, means a half-finished edit is less likely to be upstream before the next write finishes it, but the repositories this is for are the ones where that does not matter. Five seconds outlasts one agent's burst of writes while being an order of magnitude faster than a timer. `gitwatch`'s two seconds was judged too eager. Set it per repository for anything that is executed rather than read.
- **Pulling is on.** A rejected push runs `git pull --rebase --autostash` and pushes again; a conflict aborts the rebase and asks for a person rather than retrying. That is enough for a second machine writing the same repository, as far as a rejected push goes.
- **An in-process recheck**, ten minutes, retries a push while commits remain unpushed even when no file changes.
- **A `once` subcommand** runs one pass over every repository and exits non-zero on failure, for a backstop timer or a one-off.

## Not yet decided

- Whether a long-interval backstop timer belongs alongside the daemon. In-process retry covers a failed push, and the process exiting covers a dead watcher, but only if something restarts it. A timer calling `boswell once` is cheap insurance; the alternative is `Restart=` and a watchdog on the unit.

## State

Released. The daemon, config format, tests, CI and the tag-triggered release building a static musl binary all exist. What remains is on the side of whoever deploys it: a unit file, and retiring the timer it replaces.
