use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use log::{info, warn};
use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

use crate::config::{Config, Repo, Retry};
use crate::git::Git;
use crate::issue::{Reporter, TokenSource};
use crate::sync::{ThreadSleeper, sync_repo};
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
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(COALESCE, None, tx)
        .map_err(|e| format!("cannot watch {}: {e}", repo.path.display()))?;
    debouncer
        .watch(&repo.path, RecursiveMode::Recursive)
        .map_err(|e| format!("cannot watch {}: {e}", repo.path.display()))?;
    info!("watching {}", repo.path.display());

    // The catch-up pass runs after registration, not before: a write that lands
    // while it is running then queues on the channel, instead of waiting for
    // some unrelated later change to notice it.
    let git = Git::new(&repo.path);
    let sleeper = ThreadSleeper;
    let mut outstanding = sync_repo(&git, repo, retry, reporter, &sleeper).is_failure();

    let git_dir = repo.path.join(".git");
    loop {
        let event = if outstanding {
            match rx.recv_timeout(repo.recheck) {
                Ok(result) => Some(result),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(result) => Some(result),
                Err(_) => break,
            }
        };
        if let Some(result) = event {
            if !is_relevant(result, &git_dir) {
                continue;
            }
            if !settle(&rx, &git_dir, repo.debounce) {
                break;
            }
        }
        outstanding = sync_repo(&git, repo, retry, reporter, &sleeper).is_failure();
    }
    Err(format!("the watcher for {} stopped", repo.path.display()).into())
}

/// Waits for `debounce` of quiet, restarting on every relevant event, so a
/// burst of writes becomes one commit. False means the watcher went away.
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
        Ok(events) => events
            .iter()
            .any(|event| event.paths.iter().any(|path| !under(path, git_dir))),
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
