use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn message(&self) -> String {
        let combined = format!("{}\n{}", self.stderr.trim_end(), self.stdout.trim_end());
        combined.trim().to_string()
    }
}

pub struct Git {
    dir: PathBuf,
    env: Vec<(String, String)>,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            env: Vec::new(),
        }
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn run(&self, args: &[&str]) -> io::Result<Output> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.dir).args(args);
        command.env("GIT_TERMINAL_PROMPT", "0");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let out = command.output()?;
        Ok(Output {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn stdout(&self, args: &[&str]) -> String {
        self.run(args)
            .map(|o| if o.success { o.stdout } else { String::new() })
            .unwrap_or_default()
    }

    fn lines(&self, args: &[&str]) -> Vec<String> {
        self.stdout(args)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }

    pub fn is_work_tree(&self) -> bool {
        self.stdout(&["rev-parse", "--is-inside-work-tree"]).trim() == "true"
    }

    pub fn branch(&self) -> io::Result<String> {
        let out = self.run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        if out.success {
            Ok(out.stdout.trim().to_string())
        } else {
            Err(io::Error::other(out.message()))
        }
    }

    pub fn has_upstream(&self) -> bool {
        self.run(&[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ])
        .map(|o| o.success)
        .unwrap_or(false)
    }

    pub fn status_porcelain(&self) -> Vec<String> {
        self.lines(&["status", "--porcelain"])
    }

    pub fn unpushed(&self) -> Vec<String> {
        self.lines(&["log", "--oneline", "@{upstream}..HEAD"])
    }

    pub fn behind(&self) -> Vec<String> {
        self.lines(&["log", "--oneline", "HEAD..@{upstream}"])
    }

    pub fn staged_files(&self) -> Vec<String> {
        self.lines(&["diff", "--cached", "--name-only"])
    }

    pub fn add_all(&self) -> io::Result<Output> {
        self.run(&["add", "-A"])
    }

    pub fn commit(&self, subject: &str) -> io::Result<Output> {
        self.run(&["commit", "-m", subject])
    }

    pub fn push(&self, remote: &str, branch: &str, set_upstream: bool) -> io::Result<Output> {
        let mut args = vec!["push"];
        if set_upstream {
            args.push("--set-upstream");
        }
        args.push(remote);
        args.push(branch);
        self.run(&args)
    }

    pub fn fetch(&self, remote: &str) -> io::Result<Output> {
        self.run(&["fetch", remote])
    }

    pub fn pull_rebase(&self, remote: &str, branch: &str) -> io::Result<Output> {
        self.run(&["pull", "--rebase", "--autostash", remote, branch])
    }

    pub fn rebase_abort(&self) -> io::Result<Output> {
        self.run(&["rebase", "--abort"])
    }

    /// The URL as configured, not as `git remote get-url` reports it: that
    /// applies `url.*.insteadOf` rewriting, which can turn a GitHub remote into
    /// whatever transport the machine substitutes.
    pub fn remote_url(&self, remote: &str) -> io::Result<String> {
        let key = format!("remote.{remote}.url");
        let out = self.run(&["config", "--get", &key])?;
        if out.success && !out.stdout.trim().is_empty() {
            return Ok(out.stdout.trim().to_string());
        }
        let out = self.run(&["remote", "get-url", remote])?;
        if out.success {
            Ok(out.stdout.trim().to_string())
        } else {
            Err(io::Error::other(out.message()))
        }
    }
}
