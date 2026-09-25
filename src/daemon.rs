use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use log::{info, warn};
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

use crate::config::{Config, Repo, Retry};
use crate::git::Git;
use crate::issue::{Reporter, TITLE, TokenSource};
use crate::sync::{Outcome, ThreadSleeper, sync_repo};
use crate::{Error, Result};

/// Only coalesces raw inotify chatter; the debounce the config asks for is the
/// deadline loop below, which has to be restartable by each new event.
const COALESCE: Duration = Duration::from_millis(250);

/// Returns as soon as any one repository stops being watched. Joining the
/// threads in order would hide a dead repository behind a live one, and a
/// daemon watching half of what it was asked to is worse than a dead one: this
/// way the process exits non-zero and whatever supervises it can restart.
pub fn run(config: Config) -> Result<()> {
    let config = Arc::new(config);
    let reporter = Arc::new(Reporter::new(
        &config.github.api_url,
        TokenSource::Environment,
    ));
    let (tx, rx) = channel::<Error>();
    for index in 0..config.repos.len() {
        let config = Arc::clone(&config);
        let reporter = Arc::clone(&reporter);
        let tx = tx.clone();
        std::thread::spawn(move || {
            let repo = &config.repos[index];
            let attempt = catch_unwind(AssertUnwindSafe(|| watch(repo, &config.retry, &reporter)));
            let error: Error = match attempt {
                Ok(Ok(())) => format!("{} stopped being watched", repo.path.display()).into(),
                Ok(Err(e)) => e,
                Err(_) => format!("the watcher for {} panicked", repo.path.display()).into(),
            };
            let _ = tx.send(error);
        });
    }
    drop(tx);
    match rx.recv() {
        Ok(error) => Err(error),
        Err(_) => Err("no repositories left to watch".into()),
    }
}

pub fn once(config: &Config) -> bool {
    let reporter = Reporter::new(&config.github.api_url, TokenSource::Environment);
    let mut all_well = true;
    for repo in &config.repos {
        let git = Git::new(&repo.path);
        let outcome = sync_repo(&git, repo, &config.retry, &reporter, &ThreadSleeper);
        all_well &= !outcome.is_failure();
    }
    all_well
}

fn watch(repo: &Repo, retry: &Retry, reporter: &Reporter) -> Result<()> {
    // FSEvents reports canonical paths, so a root reached through a symlink
    // comes back spelled the other way: `/var/folders/x` as
    // `/private/var/folders/x`. The `.git` prefix below has to be the spelling
    // the events arrive in or nothing is filtered, and every pass's own writes
    // under `.git` start the next one, for ever.
    let root = repo
        .path
        .canonicalize()
        .map_err(|e| format!("cannot watch {}: {e}", repo.path.display()))?;
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(COALESCE, None, tx)
        .map_err(|e| format!("cannot watch {}: {e}", repo.path.display()))?;
    debouncer
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("cannot watch {}: {e}", repo.path.display()))?;
    info!("watching {}", root.display());

    // The catch-up pass runs after registration, not before: a write that lands
    // while it is running then queues on the channel, instead of waiting for
    // some unrelated later change to notice it.
    let git = Git::new(&repo.path);
    let sleeper = ThreadSleeper;
    let fetch_interval =
        (repo.pull && !repo.fetch_interval.is_zero()).then_some(repo.fetch_interval);
    let outcome = sync_repo(&git, repo, retry, reporter, &sleeper);
    let mut outstanding = outcome.is_failure();
    let mut paused = outcome == Outcome::NeedsHuman;
    let mut last_sync = Instant::now();
    let mut fetch_due = fetch_interval.and_then(|interval| last_sync.checked_add(interval));

    let git_dir = root.join(".git");
    loop {
        // A deadline that does not fit in an `Instant` is a configured interval
        // so long that never waking on it is the same thing.
        let recheck_due = outstanding
            .then(|| last_sync.checked_add(repo.recheck))
            .flatten();
        let due = [recheck_due, fetch_due].into_iter().flatten().min();
        let event = match due {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(result) => Some(result),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(result) => Some(result),
                Err(_) => break,
            },
        };
        match event {
            Some(result) => {
                if !is_relevant(result, &git_dir) {
                    continue;
                }
                if !settle(&rx, &git_dir, repo.debounce) {
                    break;
                }
            }
            // Rebasing again over the conflict that stopped the last pass would
            // only conflict again; the closed issue is the signal that a person
            // has been here. A file event still syncs, since that is new work.
            None => {
                if paused {
                    let wait = still_reported(reporter, &git, &repo.remote);
                    last_sync = Instant::now();
                    fetch_due = fetch_interval.and_then(|interval| last_sync.checked_add(interval));
                    if wait {
                        continue;
                    }
                }
            }
        }
        let outcome = sync_repo(&git, repo, retry, reporter, &sleeper);
        outstanding = outcome.is_failure();
        paused = outcome == Outcome::NeedsHuman;
        last_sync = Instant::now();
        fetch_due = fetch_interval.and_then(|interval| last_sync.checked_add(interval));
    }
    Err(format!("the watcher for {} stopped", repo.path.display()).into())
}

/// Unknown counts as still reported: a reporter that cannot answer must not
/// resume pulling into a tree nobody has looked at.
fn still_reported(reporter: &Reporter, git: &Git, remote: &str) -> bool {
    match reporter.has_open_report(git, remote) {
        Ok(open) => open,
        Err(e) => {
            warn!(
                "{}: cannot tell whether `{TITLE}` is still open: {e}",
                git.dir().display()
            );
            true
        }
    }
}

/// False means the watcher went away.
fn settle(rx: &Receiver<DebounceEventResult>, git_dir: &Path, debounce: Duration) -> bool {
    let mut deadline = Instant::now() + debounce;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        match rx.recv_timeout(deadline - now) {
            Ok(result) => {
                if is_relevant(result, git_dir) {
                    deadline = Instant::now() + debounce;
                }
            }
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn is_relevant(result: DebounceEventResult, git_dir: &Path) -> bool {
    match result {
        // A read is not a change, and every pass reads the whole tree: `git
        // status` and `git add` open each tracked file, which inotify reports
        // as an access. Counting those kept an idle repository running a
        // no-op pass every debounce, for ever. FSEvents has no access event,
        // so on macOS this drops nothing.
        Ok(events) => events.iter().any(|event| {
            !matches!(event.kind, EventKind::Access(_))
                && event.paths.iter().any(|path| !under(path, git_dir))
        }),
        Err(errors) => {
            for e in errors {
                warn!("watcher: {e}");
            }
            false
        }
    }
}

fn under(path: &Path, git_dir: &Path) -> bool {
    path.starts_with(git_dir)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::Event;
    use notify::event::{AccessKind, AccessMode, ModifyKind};
    use notify_debouncer_full::DebouncedEvent;

    use super::*;

    fn seen(kind: EventKind, path: &str) -> DebounceEventResult {
        let event = Event::new(kind).add_path(PathBuf::from(path));
        Ok(vec![DebouncedEvent::new(event, Instant::now())])
    }

    fn git_dir() -> PathBuf {
        PathBuf::from("/repo/.git")
    }

    #[test]
    fn a_write_in_the_tree_starts_a_pass() {
        assert!(is_relevant(
            seen(EventKind::Modify(ModifyKind::Any), "/repo/a.md"),
            &git_dir()
        ));
    }

    #[test]
    fn a_write_under_dot_git_does_not() {
        assert!(!is_relevant(
            seen(EventKind::Modify(ModifyKind::Any), "/repo/.git/index"),
            &git_dir()
        ));
    }

    #[test]
    fn a_read_of_a_tracked_file_does_not() {
        assert!(!is_relevant(
            seen(
                EventKind::Access(AccessKind::Open(AccessMode::Any)),
                "/repo/a.md"
            ),
            &git_dir()
        ));
    }
}
