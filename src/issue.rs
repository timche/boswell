use std::process::Command;

use log::{error, info};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::git::Git;

pub const TITLE: &str = "Auto-sync failed";

const ERROR_LINES: usize = 40;
const COMMITTED_LINES: usize = 20;
const UNCOMMITTED_LINES: usize = 40;

#[derive(Debug, Clone)]
pub enum TokenSource {
    Environment,
    Fixed(String),
}

impl TokenSource {
    fn resolve(&self) -> Option<String> {
        match self {
            TokenSource::Fixed(token) => Some(token.clone()),
            TokenSource::Environment => env_token().or_else(gh_cli_token),
        }
    }
}

fn env_token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
}

fn gh_cli_token() -> Option<String> {
    let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

pub fn slug_from_remote_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = [
        "https://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
    let slug = rest.strip_suffix(".git").unwrap_or(rest).trim_matches('/');
    (!slug.is_empty()).then(|| slug.to_string())
}

#[derive(Deserialize)]
struct IssueSummary {
    title: String,
}

#[derive(Serialize)]
struct NewIssue<'a> {
    title: &'a str,
    body: &'a str,
}

pub struct Reporter {
    api_url: String,
    token: TokenSource,
}

impl Reporter {
    pub fn new(api_url: impl Into<String>, token: TokenSource) -> Self {
        Self {
            api_url: api_url.into().trim_end_matches('/').to_string(),
            token,
        }
    }

    /// Never fails the caller: a daemon that cannot report must still keep watching.
    pub fn report(&self, git: &Git, remote: &str, error_output: &str) {
        if let Err(e) = self.try_report(git, remote, error_output) {
            error!("could not file an issue for {}: {e}", git.dir().display());
        }
    }

    /// Whether the repository still carries an open `Auto-sync failed`. The
    /// error is the caller's to interpret: not knowing is not the same as no.
    pub fn has_open_report(&self, git: &Git, remote: &str) -> Result<bool> {
        let (slug, token) = self.slug_and_token(git, remote)?;
        self.query_open_report(&slug, &token)
    }

    fn slug_and_token(&self, git: &Git, remote: &str) -> Result<(String, String)> {
        let url = git.remote_url(remote)?;
        let slug = slug_from_remote_url(&url)
            .ok_or_else(|| format!("remote {remote} is not a GitHub repository: {url}"))?;
        let Some(token) = self.token.resolve() else {
            return Err("no GitHub token in GH_TOKEN, GITHUB_TOKEN or `gh auth token`".into());
        };
        Ok((slug, token))
    }

    fn try_report(&self, git: &Git, remote: &str, error_output: &str) -> Result<()> {
        let (slug, token) = self.slug_and_token(git, remote)?;
        if self.query_open_report(&slug, &token)? {
            info!("{slug} already has an open `{TITLE}` issue; not filing another");
            return Ok(());
        }
        let body = issue_body(git, remote, error_output);
        self.file(&slug, &token, &body)?;
        info!("filed `{TITLE}` against {slug}");
        Ok(())
    }

    fn query_open_report(&self, slug: &str, token: &str) -> Result<bool> {
        let url = format!(
            "{}/repos/{slug}/issues?state=open&per_page=100",
            self.api_url
        );
        let mut response = self.request(ureq::get(&url), token).call()?;
        let issues: Vec<IssueSummary> = response.body_mut().read_json()?;
        Ok(issues.iter().any(|issue| issue.title == TITLE))
    }

    fn file(&self, slug: &str, token: &str, body: &str) -> Result<()> {
        let url = format!("{}/repos/{slug}/issues", self.api_url);
        self.request(ureq::post(&url), token)
            .send_json(NewIssue { title: TITLE, body })?;
        Ok(())
    }

    fn request<T>(&self, builder: ureq::RequestBuilder<T>, token: &str) -> ureq::RequestBuilder<T> {
        builder
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "boswell")
    }
}

fn hostname() -> String {
    for path in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(name) = std::fs::read_to_string(path) {
            let name = name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    "an unknown host".to_string()
}

fn fenced(lines: &[String], limit: usize) -> String {
    if lines.is_empty() {
        return "```\n(nothing)\n```".to_string();
    }
    let shown: Vec<&str> = lines.iter().take(limit).map(String::as_str).collect();
    format!("```\n{}\n```", shown.join("\n"))
}

fn issue_body(git: &Git, remote: &str, error_output: &str) -> String {
    let path = git.dir().display();
    let error = if error_output.trim().is_empty() {
        vec![]
    } else {
        error_output.trim().lines().map(str::to_string).collect()
    };
    format!(
        "`{path}` could not be pushed to `{remote}` on {host}.

Nothing written there since is upstream, and nothing new will reach the origin until this is fixed by hand.

Fix the push, then **close this issue**: while it is open boswell suppresses the next report, so leaving it open hides the following failure.

## Last push or pull error

{error}

## Committed but not pushed

{committed}

## Uncommitted

{uncommitted}
",
        host = hostname(),
        error = fenced(&error, ERROR_LINES),
        committed = fenced(&git.unpushed(), COMMITTED_LINES),
        uncommitted = fenced(&git.status_porcelain(), UNCOMMITTED_LINES),
    )
}

#[cfg(test)]
mod tests {
    use super::slug_from_remote_url;

    #[test]
    fn https_with_git_suffix() {
        assert_eq!(
            slug_from_remote_url("https://github.com/o/r.git").as_deref(),
            Some("o/r")
        );
    }

    #[test]
    fn ssh_with_git_suffix() {
        assert_eq!(
            slug_from_remote_url("git@github.com:o/r.git").as_deref(),
            Some("o/r")
        );
    }

    #[test]
    fn https_without_git_suffix() {
        assert_eq!(
            slug_from_remote_url("https://github.com/o/r").as_deref(),
            Some("o/r")
        );
    }

    #[test]
    fn ssh_url_form() {
        assert_eq!(
            slug_from_remote_url("ssh://git@github.com/o/r.git").as_deref(),
            Some("o/r")
        );
        assert_eq!(
            slug_from_remote_url("ssh://git@github.com/o/r").as_deref(),
            Some("o/r")
        );
    }

    #[test]
    fn a_local_path_has_no_slug() {
        assert_eq!(slug_from_remote_url("/tmp/remote.git"), None);
    }
}
