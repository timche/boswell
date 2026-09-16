mod support;

use std::time::Duration;

use boswell::issue::{Reporter, TokenSource, slug_from_remote_url};
use boswell::sync::{Outcome, sync_repo};
use support::{Fixture, Recorder, Stub, retry};

fn reporter(stub: &Stub) -> Reporter {
    let _ = env_logger::builder().is_test(true).try_init();
    Reporter::new(&stub.url, TokenSource::Fixed("test".to_string()))
}

fn quick() -> boswell::config::Retry {
    retry(3, Duration::from_millis(1), Duration::from_millis(1))
}

#[test]
fn a_dirty_tree_becomes_one_commit_on_the_remote() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.write("a.md", "a\n");
    fixture.write("b.md", "b\n");

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &quick(),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(
        outcome,
        Outcome::Pushed {
            subject: Some("Update a.md, b.md".to_string())
        }
    );
    assert_eq!(
        fixture.remote_subjects(),
        vec!["Update a.md, b.md".to_string(), "first".to_string()]
    );
    assert!(stub.posts().is_empty());
}

#[test]
fn a_clean_tree_with_nothing_unpushed_does_nothing() {
    let fixture = Fixture::new();
    let stub = Stub::start();

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &quick(),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(outcome, Outcome::Nothing);
    assert_eq!(fixture.local_subjects(), vec!["first".to_string()]);
    assert_eq!(fixture.remote_subjects(), vec!["first".to_string()]);
}

#[test]
fn an_unpushed_commit_is_pushed_without_a_new_one() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.write("a.md", "a\n");
    fixture.git(&["add", "-A"]);
    fixture.git(&["commit", "-m", "by hand"]);

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &quick(),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(outcome, Outcome::Pushed { subject: None });
    assert_eq!(
        fixture.remote_subjects(),
        vec!["by hand".to_string(), "first".to_string()]
    );
}

#[test]
fn a_rejected_push_rebases_and_lands_both_commits() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.advance_remote("theirs.md", "theirs\n", "from elsewhere");
    fixture.write("mine.md", "mine\n");

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &quick(),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(
        outcome,
        Outcome::Pushed {
            subject: Some("Update mine.md".to_string())
        }
    );
    assert_eq!(
        fixture.remote_subjects(),
        vec![
            "Update mine.md".to_string(),
            "from elsewhere".to_string(),
            "first".to_string()
        ]
    );
    assert!(stub.posts().is_empty());
}

#[test]
fn a_rebase_on_the_last_attempt_still_gets_its_push() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.advance_remote("theirs.md", "theirs\n", "from elsewhere");
    fixture.write("mine.md", "mine\n");

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &retry(1, Duration::from_millis(1), Duration::from_millis(1)),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(
        outcome,
        Outcome::Pushed {
            subject: Some("Update mine.md".to_string())
        }
    );
    assert_eq!(
        fixture.remote_subjects(),
        vec![
            "Update mine.md".to_string(),
            "from elsewhere".to_string(),
            "first".to_string()
        ]
    );
    assert!(stub.posts().is_empty());
}

#[test]
fn a_rejected_push_without_pull_needs_a_human_and_files_one_issue() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.advance_remote("theirs.md", "theirs\n", "from elsewhere");
    fixture.write("mine.md", "mine\n");

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(false),
        &quick(),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(outcome, Outcome::NeedsHuman);
    let posts = stub.posts();
    assert_eq!(posts.len(), 1, "expected exactly one issue, got {posts:?}");
    assert!(posts[0].contains("Auto-sync failed"), "{}", posts[0]);
    assert!(
        posts[0].contains(fixture.work.to_str().unwrap()),
        "{}",
        posts[0]
    );
}

#[test]
fn a_rebase_conflict_is_aborted_and_reported() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.advance_remote("README.md", "theirs\n", "their edit");
    fixture.write("README.md", "mine\n");

    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &quick(),
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(outcome, Outcome::NeedsHuman);
    assert!(!fixture.work.join(".git/rebase-merge").exists());
    assert!(!fixture.work.join(".git/rebase-apply").exists());
    assert_eq!(
        fixture.local_subjects(),
        vec!["Update README.md".to_string(), "first".to_string()]
    );
    assert_eq!(stub.posts().len(), 1);
}

#[test]
fn an_unreachable_remote_backs_off_then_reports_once() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    fixture.break_transport();
    fixture.write("a.md", "a\n");

    let retry = retry(4, Duration::from_secs(1), Duration::from_secs(3));
    let recorder = Recorder::default();
    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &retry,
        &reporter(&stub),
        &recorder,
    );

    assert_eq!(outcome, Outcome::Unreachable);
    assert_eq!(
        recorder.delays(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(3)
        ]
    );
    assert_eq!(stub.posts().len(), 1);

    stub.report_open_issue(true);
    let outcome = sync_repo(
        &fixture.repo_git(),
        &fixture.repo(true),
        &retry,
        &reporter(&stub),
        &Recorder::default(),
    );

    assert_eq!(outcome, Outcome::Unreachable);
    assert_eq!(
        stub.posts().len(),
        1,
        "an open issue must suppress the next report"
    );
}

#[test]
fn slugs_come_from_either_github_remote_spelling() {
    assert_eq!(
        slug_from_remote_url("https://github.com/o/r.git").as_deref(),
        Some("o/r")
    );
    assert_eq!(
        slug_from_remote_url("git@github.com:o/r.git").as_deref(),
        Some("o/r")
    );
    assert_eq!(
        slug_from_remote_url("https://github.com/o/r").as_deref(),
        Some("o/r")
    );
    assert_eq!(
        slug_from_remote_url("ssh://git@github.com/o/r.git").as_deref(),
        Some("o/r")
    );
}
