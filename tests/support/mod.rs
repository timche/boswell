#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use boswell::config::{Repo, Retry};
use boswell::git::Git;
use boswell::sync::Sleeper;
use tempfile::TempDir;

pub const SLUG: &str = "boswell-test/repo";
pub const ORIGIN_URL: &str = "https://github.com/boswell-test/repo.git";
pub const BRANCH: &str = "main";

pub struct Fixture {
    pub dir: TempDir,
    pub remote: PathBuf,
    pub work: PathBuf,
    no_config: PathBuf,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().to_path_buf();
        let fixture = Fixture {
            remote: root.join("remote.git"),
            work: root.join("work"),
            no_config: root.join("no-such-git-config"),
            dir,
        };
        run(
            &root,
            &fixture.no_config,
            &["init", "--bare", "-b", BRANCH, "remote.git"],
        );
        fixture.clone_into(&fixture.work);
        fixture.write("README.md", "one\n");
        fixture.git(&["add", "-A"]);
        fixture.git(&["commit", "-m", "first"]);
        fixture.git(&["push", "-u", "origin", BRANCH]);
        fixture
    }

    /// `origin` is spelled as a GitHub URL so the issue filer can derive a
    /// slug; `insteadOf` sends the transport to the bare repository next door
    /// instead.
    pub fn clone_into(&self, path: &Path) {
        run(
            self.dir.path(),
            &self.no_config,
            &["init", "-b", BRANCH, path.to_str().unwrap()],
        );
        let at = |args: &[&str]| run(path, &self.no_config, args);
        at(&["config", "user.name", "Boswell Test"]);
        at(&["config", "user.email", "boswell@example.invalid"]);
        at(&["remote", "add", "origin", ORIGIN_URL]);
        at(&[
            "config",
            &format!("url.{}.insteadOf", self.remote.display()),
            ORIGIN_URL,
        ]);
    }

    pub fn second_clone(&self) -> PathBuf {
        let path = self.dir.path().join("other");
        self.clone_into(&path);
        run(&path, &self.no_config, &["fetch", "origin"]);
        run(
            &path,
            &self.no_config,
            &["reset", "--hard", &format!("origin/{BRANCH}")],
        );
        run(
            &path,
            &self.no_config,
            &["branch", "--set-upstream-to", &format!("origin/{BRANCH}")],
        );
        path
    }

    pub fn git(&self, args: &[&str]) -> String {
        run(&self.work, &self.no_config, args)
    }

    pub fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        run(dir, &self.no_config, args)
    }

    pub fn write(&self, name: &str, contents: &str) {
        let path = self.work.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write");
    }

    /// The `Git` the library will use, with the host's git config kept out.
    pub fn repo_git(&self) -> Git {
        Git::new(&self.work)
            .with_env("GIT_CONFIG_GLOBAL", self.no_config.to_str().unwrap())
            .with_env("GIT_CONFIG_SYSTEM", self.no_config.to_str().unwrap())
    }

    pub fn no_config_path(&self) -> &Path {
        &self.no_config
    }

    pub fn remote_subjects(&self) -> Vec<String> {
        let out = run(
            self.dir.path(),
            &self.no_config,
            &[
                "--git-dir",
                self.remote.to_str().unwrap(),
                "log",
                "--format=%s",
                BRANCH,
            ],
        );
        out.lines().map(str::to_string).collect()
    }

    pub fn local_subjects(&self) -> Vec<String> {
        self.git(&["log", "--format=%s", "HEAD"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Makes `origin` resolve to a path that does not exist, without changing
    /// the URL the issue filer reads.
    pub fn break_transport(&self) {
        self.git(&[
            "config",
            "--unset",
            &format!("url.{}.insteadOf", self.remote.display()),
        ]);
        self.git(&[
            "config",
            &format!("url.{}/gone.git.insteadOf", self.dir.path().display()),
            ORIGIN_URL,
        ]);
    }

    pub fn advance_remote(&self, name: &str, contents: &str, subject: &str) {
        let other = self.second_clone();
        std::fs::write(other.join(name), contents).expect("write");
        self.git_in(&other, &["add", "-A"]);
        self.git_in(&other, &["commit", "-m", subject]);
        self.git_in(&other, &["push", "origin", BRANCH]);
    }

    pub fn repo(&self, pull: bool) -> Repo {
        Repo {
            path: self.work.clone(),
            debounce: Duration::from_millis(50),
            remote: "origin".to_string(),
            pull,
            recheck: Duration::from_secs(600),
            fetch_interval: Duration::from_secs(600),
        }
    }

    pub fn repo_fetching(&self, interval: Duration) -> Repo {
        Repo {
            fetch_interval: interval,
            ..self.repo(true)
        }
    }
}

fn run(dir: &Path, no_config: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", no_config)
        .env("GIT_CONFIG_SYSTEM", no_config)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "fixture `git {}` in {} failed: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn retry(attempts: u32, base: Duration, max: Duration) -> Retry {
    Retry {
        attempts,
        base,
        max,
    }
}

#[derive(Default)]
pub struct Recorder {
    delays: Mutex<Vec<Duration>>,
}

impl Recorder {
    pub fn delays(&self) -> Vec<Duration> {
        self.delays.lock().unwrap().clone()
    }
}

impl Sleeper for Recorder {
    fn sleep(&self, delay: Duration) {
        self.delays.lock().unwrap().push(delay);
    }
}

struct StubState {
    posts: Mutex<Vec<String>>,
    open_issue: AtomicBool,
    stop: AtomicBool,
}

pub struct Stub {
    pub url: String,
    state: Arc<StubState>,
    handle: Option<JoinHandle<()>>,
}

impl Stub {
    pub fn start() -> Stub {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("stub api");
        let url = format!("http://{}", server.server_addr());
        let state = Arc::new(StubState {
            posts: Mutex::new(Vec::new()),
            open_issue: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let served = Arc::clone(&state);
        let handle = std::thread::spawn(move || serve(server, served));
        Stub {
            url,
            state,
            handle: Some(handle),
        }
    }

    pub fn report_open_issue(&self, open: bool) {
        self.state.open_issue.store(open, Ordering::SeqCst);
    }

    pub fn posts(&self) -> Vec<String> {
        self.state.posts.lock().unwrap().clone()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(server: tiny_http::Server, state: Arc<StubState>) {
    let json = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("header");
    while !state.stop.load(Ordering::SeqCst) {
        let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        let response = if request.method() == &tiny_http::Method::Post {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            state.posts.lock().unwrap().push(body);
            tiny_http::Response::from_string("{\"number\":1}").with_status_code(201)
        } else if state.open_issue.load(Ordering::SeqCst) {
            tiny_http::Response::from_string("[{\"title\":\"Auto-sync failed\"}]")
        } else {
            tiny_http::Response::from_string("[]")
        };
        let _ = request.respond(response.with_header(json.clone()));
    }
}
