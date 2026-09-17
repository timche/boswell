# boswell

A daemon that watches git repositories and commits and pushes what changes. `docs/design.md` is the design and the record of what was decided and what is still open; read it before changing behaviour, and update it when a decision changes. `README.md` is the user-facing install, configure and run guide and has to keep matching the config the code accepts.

## Working here

- Rust is pinned in `mise.toml`; `mise install` brings rustfmt and clippy with it. Before finishing: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. CI runs the same three.
- Shell out to `git`, never a git library. No async runtime. Keep the dependency list to what the design names.
- Commit subjects are lowercase and imperative and say what changed and why it matters, like the history.
- A release is a `v*` tag: bump `version` in `Cargo.toml` first, then tag and push. The workflow builds the static musl binary that `github:timche/boswell` installs.

## Layout

- `src/sync.rs` is one sync pass: add, commit, push with retry, and the three push outcomes. `src/daemon.rs` watches and debounces and calls it. `src/issue.rs` files the `Auto-sync failed` issue. `src/subject.rs` is the commit subject.
- `tests/support/mod.rs` builds a temporary repository with a bare remote and a stub GitHub API; every integration test starts from it.

## Shipping

- No pull requests. Merge a finished, reviewed branch into `main` with a fast-forward and push. Release by tagging as above.
