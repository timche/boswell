use std::time::Duration;

use log::{error, info, warn};

use crate::config::{Repo, Retry};
use crate::git::Git;
use crate::issue::Reporter;
use crate::subject::commit_subject;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Pulled,
    Pushed { subject: Option<String> },
    NeedsHuman,
    Unreachable,
}

impl Outcome {
    pub fn is_failure(&self) -> bool {
        matches!(self, Outcome::NeedsHuman | Outcome::Unreachable)
    }
}

pub trait Sleeper: Send + Sync {
    fn sleep(&self, delay: Duration);
}

pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&self, delay: Duration) {
        std::thread::sleep(delay);
    }
}

/// Deliberately not a bare `rejected`: `! [remote rejected]` is a hook or a
/// protected branch refusing the push, which no amount of rebasing fixes.
fn is_rejection(stderr: &str) -> bool {
    let stderr = stderr.to_lowercase();
    ["[rejected]", "fetch first", "non-fast-forward"]
        .iter()
        .any(|marker| stderr.contains(marker))
}

pub fn sync_repo(
    git: &Git,
    repo: &Repo,
    retry: &Retry,
    reporter: &Reporter,
    sleeper: &dyn Sleeper,
) -> Outcome {
    match try_sync(git, repo, retry, sleeper) {
        Ok((outcome, last_error)) => {
            if outcome.is_failure() {
                error!("{}: {last_error}", git.dir().display());
                reporter.report(git, &repo.remote, &last_error);
            }
            outcome
        }
        Err(message) => {
            error!("{}: {message}", git.dir().display());
            reporter.report(git, &repo.remote, &message);
            Outcome::Unreachable
        }
    }
}

fn try_sync(
    git: &Git,
    repo: &Repo,
    retry: &Retry,
    sleeper: &dyn Sleeper,
) -> Result<(Outcome, String), String> {
    let branch = git.branch().map_err(|e| e.to_string())?;
    let mut set_upstream = !git.has_upstream();
    let dirty = !git.status_porcelain().is_empty();
    // Without an upstream the branch has never been published, so there is
    // always something to push even when `@{upstream}..HEAD` cannot be asked.
    let unpushed = set_upstream || !git.unpushed().is_empty();
    // With pulling on, an idle repository is still worth a pass: the remote may
    // have moved even though nothing here did.
    if !repo.pull && !dirty && !unpushed {
        return Ok((Outcome::Nothing, String::new()));
    }

    // Committing before the loop is what makes the rebase below safe: it runs
    // over a checkpoint rather than over half-written work.
    let mut subject = None;
    git.add_all().map_err(|e| e.to_string())?;
    let staged = git.staged_files();
    if !staged.is_empty() {
        let message = commit_subject(&staged);
        let out = git.commit(&message).map_err(|e| e.to_string())?;
        if !out.success {
            return Err(format!("commit failed: {}", out.message()));
        }
        subject = Some(message);
    }

    let mut delay = retry.base;
    let mut last_error = String::new();
    let mut pulled = 0;
    for attempt in 1..=retry.attempts {
        // An unpublished branch has no upstream to fetch against and nothing
        // upstream to rebase onto.
        if repo.pull && !set_upstream {
            let fetched = git.fetch(&repo.remote).map_err(|e| e.to_string())?;
            if !fetched.success {
                // An offline machine with nothing of its own to send is not a
                // failure: retrying the ladder and filing an issue every minute
                // would make ordinary idleness look broken.
                if git.unpushed().is_empty() {
                    warn!(
                        "{}: cannot fetch, and there is nothing to push: {}",
                        git.dir().display(),
                        fetched.message()
                    );
                    return Ok((Outcome::Nothing, String::new()));
                }
                last_error = fetched.message();
                if attempt < retry.attempts {
                    sleeper.sleep(delay);
                    delay = (delay * 2).min(retry.max);
                }
                continue;
            }
            let behind = git.behind();
            if !behind.is_empty() {
                let pull = git
                    .pull_rebase(&repo.remote, &branch)
                    .map_err(|e| e.to_string())?;
                if !pull.success {
                    // Leave no half-finished rebase behind: whoever fixes this
                    // by hand should find an ordinary working tree.
                    if let Err(e) = git.rebase_abort() {
                        warn!("{}: rebase --abort failed: {e}", git.dir().display());
                    }
                    return Ok((Outcome::NeedsHuman, pull.message()));
                }
                pulled += behind.len();
            }
        }

        if !set_upstream && git.unpushed().is_empty() {
            if pulled > 0 {
                info!("{}: pulled {pulled} commits", git.dir().display());
                return Ok((Outcome::Pulled, String::new()));
            }
            // No log line: with the fetch interval on, this is the common case
            // and it happens every minute.
            return Ok((Outcome::Nothing, String::new()));
        }

        let mut out = git
            .push(&repo.remote, &branch, set_upstream)
            .map_err(|e| e.to_string())?;

        // Kept even with the fetch above: something can land between the two.
        if !out.success && is_rejection(&out.message()) {
            if !repo.pull {
                return Ok((Outcome::NeedsHuman, out.message()));
            }
            let pull = git
                .pull_rebase(&repo.remote, &branch)
                .map_err(|e| e.to_string())?;
            if !pull.success {
                if let Err(e) = git.rebase_abort() {
                    warn!("{}: rebase --abort failed: {e}", git.dir().display());
                }
                return Ok((Outcome::NeedsHuman, pull.message()));
            }
            set_upstream = false;
            // The push has to follow the rebase here rather than on the next
            // attempt, which on the last one would never come.
            out = git
                .push(&repo.remote, &branch, false)
                .map_err(|e| e.to_string())?;
            if !out.success && is_rejection(&out.message()) {
                // Rejected again on a freshly rebased branch: something else is
                // writing to it, and retrying would only lose whichever race.
                return Ok((Outcome::NeedsHuman, out.message()));
            }
        }

        if out.success {
            match &subject {
                Some(s) => info!("{}: pushed `{s}`", git.dir().display()),
                None => info!("{}: pushed already-committed work", git.dir().display()),
            }
            return Ok((Outcome::Pushed { subject }, String::new()));
        }
        last_error = out.message();

        if attempt < retry.attempts {
            sleeper.sleep(delay);
            delay = (delay * 2).min(retry.max);
        }
    }
    Ok((Outcome::Unreachable, last_error))
}
