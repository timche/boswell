mod support;

use std::process::{Child, Command};
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
