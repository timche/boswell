mod support;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use support::{Fixture, Stub};

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + deadline;
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

#[test]
fn an_idle_repository_stops_passing() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    // One failing pass per wake, cheaply, and an open issue so the failures are
    // never filed: what is being counted is the passes themselves.
    fixture.git(&["commit", "--allow-empty", "-m", "by hand"]);
    fixture.break_transport();
    stub.report_open_issue(true);

    let config = fixture.dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[github]\napi_url = \"{}\"\n\n[retry]\nattempts = 1\nbase = \"1ms\"\nmax = \"1ms\"\n\n[[repo]]\npath = \"{}\"\ndebounce = \"300ms\"\n",
            stub.url,
            fixture.work.display()
        ),
    )
    .expect("config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_boswell"))
        .arg("--config")
        .arg(&config)
        .env("GH_TOKEN", "test")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", fixture.no_config_path())
        .env("GIT_CONFIG_SYSTEM", fixture.no_config_path())
        .env("RUST_LOG", "info")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn boswell");
    let lines = stderr_lines(&mut child);
    let _running = Running(child);

    assert!(
        wait_for_line(&lines, "ERROR", Duration::from_secs(15)),
        "the catch-up pass should have failed against the broken remote"
    );
    // That pass read every tracked file. If those reads counted as changes it
    // would be followed by another pass a debounce later, and another after
    // that, with nobody touching the tree.
    assert!(
        !wait_for_line(&lines, "ERROR", Duration::from_secs(2)),
        "a pass ran again with nothing written to the tree"
    );
}

#[test]
fn the_fetch_tick_brings_another_machines_commit_down() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    let config = fixture.dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[github]\napi_url = \"{}\"\n\n[[repo]]\npath = \"{}\"\ndebounce = \"300ms\"\nfetch_interval = \"3s\"\n",
            stub.url,
            fixture.work.display()
        ),
    )
    .expect("config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_boswell"))
        .arg("--config")
        .arg(&config)
        .env("GH_TOKEN", "test")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", fixture.no_config_path())
        .env("GIT_CONFIG_SYSTEM", fixture.no_config_path())
        .env("RUST_LOG", "info")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn boswell");
    let lines = stderr_lines(&mut child);
    let _running = Running(child);

    assert!(
        wait_for_line(
            &lines,
            &format!("watching {}", fixture.work.display()),
            Duration::from_secs(15)
        ),
        "boswell never reported that it was watching"
    );
    // `watching` is logged before the catch-up pass, so a push of our own is
    // what proves that pass is behind us and cannot be what pulls. The commit
    // below is then made through a clone outside the watched tree, so no file
    // event of its own can start a pass.
    fixture.write("mine.md", "mine\n");
    assert!(
        wait_for_line(&lines, "pushed `Update mine.md`", Duration::from_secs(15)),
        "boswell never pushed the local change"
    );
    // The debouncer can deliver that write once more while the pass runs, which
    // is one more pass and no more: past it the repository is idle.
    std::thread::sleep(Duration::from_secs(1));
    let quiet_at = Instant::now();
    fixture.advance_remote("theirs.md", "theirs\n", "from elsewhere");

    assert!(
        wait_for_line(&lines, "pulled 1 commits", Duration::from_secs(15)),
        "boswell never pulled the other machine's commit"
    );
    // Nothing has written to the watched tree since, so anything sooner than
    // the next tick would mean passes are still running on their own.
    assert!(
        quiet_at.elapsed() >= Duration::from_millis(1500),
        "the pull came too soon after the remote moved to have been the fetch tick"
    );
    assert_eq!(fixture.local_subjects()[0], "from elsewhere");
    assert_eq!(fixture.remote_subjects().len(), 3);
    assert!(stub.posts().is_empty());
}

fn stderr_lines(child: &mut Child) -> Receiver<String> {
    let stderr = child.stderr.take().expect("stderr");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("{line}");
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for_line(rx: &Receiver<String>, needle: &str, deadline: Duration) -> bool {
    let until = Instant::now() + deadline;
    loop {
        let now = Instant::now();
        if now >= until {
            return false;
        }
        match rx.recv_timeout(until - now) {
            Ok(line) => {
                if line.contains(needle) {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
}

#[test]
fn a_burst_of_writes_becomes_one_commit_and_git_does_not_retrigger() {
    let fixture = Fixture::new();
    let stub = Stub::start();
    let config = fixture.dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[github]\napi_url = \"{}\"\n\n[[repo]]\npath = \"{}\"\ndebounce = \"300ms\"\n",
            stub.url,
            fixture.work.display()
        ),
    )
    .expect("config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_boswell"))
        .arg("--config")
        .arg(&config)
        .env("GH_TOKEN", "test")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", fixture.no_config_path())
        .env("GIT_CONFIG_SYSTEM", fixture.no_config_path())
        .env("RUST_LOG", "info")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn boswell");
    let lines = stderr_lines(&mut child);
    let _running = Running(child);

    assert!(
        wait_for_line(
            &lines,
            &format!("watching {}", fixture.work.display()),
            Duration::from_secs(15)
        ),
        "boswell never reported that it was watching"
    );
    // Only so the catch-up pass finishes first: a burst that straddles its
    // `git add -A` would legitimately split across two commits.
    std::thread::sleep(Duration::from_millis(300));
    fixture.write("a.md", "a\n");
    fixture.write("b.md", "b\n");

    assert!(
        wait_for(Duration::from_secs(15), || fixture.remote_subjects().len()
            == 2),
        "expected one new commit on the remote, saw {:?}",
        fixture.remote_subjects()
    );
    assert_eq!(fixture.remote_subjects()[0], "Update a.md, b.md");

    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        fixture.remote_subjects().len(),
        2,
        "git's own writes under .git must not start another sync"
    );
    assert!(stub.posts().is_empty());
}

/// Linux only. inotify takes one watch per directory, so an unreadable one
/// fails registration for the whole tree; FSEvents takes a single stream for
/// the subtree and never looks inside it, and the only path it refuses is one
/// that does not exist, which the config check rejects first. The failure this
/// test induces has no macOS equivalent.
#[cfg(target_os = "linux")]
mod unwatchable {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::ExitStatus;

    use super::*;

    /// Restores the mode itself, so the tempdir can still be removed if the test
    /// fails part way through.
    struct Locked(PathBuf);

    impl Locked {
        fn new(path: PathBuf) -> Locked {
            std::fs::create_dir_all(&path).expect("mkdir");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
            Locked(path)
        }
    }

    impl Drop for Locked {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn one_unwatchable_repository_brings_the_daemon_down() {
        let healthy = Fixture::new();
        let broken = Fixture::new();
        let stub = Stub::start();
        let _locked = Locked::new(broken.work.join("locked"));

        let config = healthy.dir.path().join("config.toml");
        std::fs::write(
            &config,
            format!(
                "[github]\napi_url = \"{}\"\n\n[[repo]]\npath = \"{}\"\n\n[[repo]]\npath = \"{}\"\n",
                stub.url,
                healthy.work.display(),
                broken.work.display()
            ),
        )
        .expect("config");

        let mut child = Command::new(env!("CARGO_BIN_EXE_boswell"))
            .arg("--config")
            .arg(&config)
            .env("GH_TOKEN", "test")
            .env("GIT_CONFIG_GLOBAL", healthy.no_config_path())
            .env("GIT_CONFIG_SYSTEM", healthy.no_config_path())
            .env("RUST_LOG", "info")
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn boswell");
        let mut stderr = child.stderr.take().expect("stderr");

        let status =
            wait_for_exit(&mut child, Duration::from_secs(20)).expect("boswell should exit");
        let mut log = String::new();
        let _ = stderr.read_to_string(&mut log);

        assert!(!status.success(), "expected a non-zero exit, got {status}");
        assert!(
            log.contains(broken.work.to_str().unwrap()),
            "the error should name the repository it could not watch:\n{log}"
        );
    }

    fn wait_for_exit(child: &mut Child, deadline: Duration) -> Option<ExitStatus> {
        let until = Instant::now() + deadline;
        while Instant::now() < until {
            match child.try_wait().expect("try_wait") {
                Some(status) => return Some(status),
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        None
    }
}
