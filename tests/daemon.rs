mod support;

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
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
    // An unreadable subdirectory fails inotify registration for the whole tree,
    // while leaving the repository itself valid at startup.
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

    let status = wait_for_exit(&mut child, Duration::from_secs(20)).expect("boswell should exit");
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

    let child = Command::new(env!("CARGO_BIN_EXE_boswell"))
        .arg("--config")
        .arg(&config)
        .env("GH_TOKEN", "test")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", fixture.no_config_path())
        .env("GIT_CONFIG_SYSTEM", fixture.no_config_path())
        .env("RUST_LOG", "info")
        .spawn()
        .expect("spawn boswell");
    let _running = Running(child);

    // The startup sync pass and the first inotify registration have to land
    // before writing, or the burst is missed rather than debounced.
    std::thread::sleep(Duration::from_secs(1));
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
