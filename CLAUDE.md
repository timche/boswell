# boswell

A daemon that watches git repositories and commits and pushes what changes. `docs/design.md` is the design and the record of what was decided and what is still open; read it before changing behaviour, and update it when a decision changes. `README.md` is the user-facing install, configure and run guide and has to keep matching the config the code accepts.

## Working here

- Rust is pinned in `mise.toml`; `mise install` brings rustfmt and clippy with it. Before finishing: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. CI runs the same three on Linux and macOS, on every branch push, which is the only way to check the FSEvents half of the watcher from a Linux machine.
- Shell out to `git`, never a git library. No async runtime. Keep the dependency list to what the design names.
- Commit subjects are lowercase and imperative and say what changed and why it matters, like the history.
- A release is a `v*` tag: bump `version` in `Cargo.toml` first, then tag and push. The workflow builds the two binaries that `github:timche/boswell` installs, static musl for x86-64 Linux and Apple Silicon for macOS, and one job downstream of both creates the release. A last job then rewrites the Homebrew formula in `timche/homebrew-tap` through `scripts/update-tap-formula.sh` and pushes it over a deploy key. To rerun that for a tag already released — a botched push, a formula fix — dispatch the `Tap` workflow with the tag; it generates the whole formula, so an unchanged one means no commit.

## Layout

- `src/sync.rs` is one sync pass: add, commit, push with retry, and the three push outcomes. `src/daemon.rs` watches and debounces and calls it. `src/issue.rs` files the `Auto-sync failed` issue. `src/subject.rs` is the commit subject.
- `tests/support/mod.rs` builds a temporary repository with a bare remote and a stub GitHub API; every integration test starts from it.

## Shipping

- No pull requests. Merge a finished, reviewed branch into `main` with a fast-forward and push.
- Every completed change is released straight away: bump `version` in `Cargo.toml` (patch for a fix, minor for new behaviour or config), commit, tag `v<version>`, push the commit and the tag. Do not leave main ahead of the last tag.
