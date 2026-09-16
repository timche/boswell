use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::Result;
use crate::git::Git;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub github: Github,
    #[serde(default)]
    pub retry: Retry,
    #[serde(default, rename = "repo")]
    pub repos: Vec<Repo>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Github {
    #[serde(default = "default_api_url")]
    pub api_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    #[serde(default = "default_attempts")]
    pub attempts: u32,
    #[serde(default = "default_base", with = "humantime_serde")]
    pub base: Duration,
    #[serde(default = "default_max", with = "humantime_serde")]
    pub max: Duration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    pub path: PathBuf,
    #[serde(default = "default_debounce", with = "humantime_serde")]
    pub debounce: Duration,
    #[serde(default = "default_remote")]
    pub remote: String,
    #[serde(default = "default_pull")]
    pub pull: bool,
    #[serde(default = "default_recheck", with = "humantime_serde")]
    pub recheck: Duration,
}

fn default_api_url() -> String {
    "https://api.github.com".to_string()
}
fn default_attempts() -> u32 {
    6
}
fn default_base() -> Duration {
    Duration::from_secs(5)
}
fn default_max() -> Duration {
    Duration::from_secs(300)
}
// Five seconds, not gitwatch's two: the docs repository tolerates early
// publication, so the window only has to outlast one agent's burst of writes.
fn default_debounce() -> Duration {
    Duration::from_secs(5)
}
fn default_remote() -> String {
    "origin".to_string()
}
fn default_pull() -> bool {
    true
}
fn default_recheck() -> Duration {
    Duration::from_secs(600)
}

impl Default for Github {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
        }
    }
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            attempts: default_attempts(),
            base: default_base(),
            max: default_max(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
        let mut config: Config = toml::from_str(&text)
            .map_err(|e| format!("cannot parse config {}: {e}", path.display()))?;
        config.prepare()?;
        Ok(config)
    }

    fn prepare(&mut self) -> Result<()> {
        if self.repos.is_empty() {
            return Err("no [[repo]] entries: boswell would watch nothing".into());
        }
        for repo in &mut self.repos {
            let expanded = expand_tilde(&repo.path);
            repo.path = expanded
                .canonicalize()
                .map_err(|e| format!("repo path {}: {e}", expanded.display()))?;
            if !Git::new(&repo.path).is_work_tree() {
                return Err(format!("{} is not a git work tree", repo.path.display()).into());
            }
        }
        Ok(())
    }
}

pub fn expand_tilde(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(home) = env::var_os("HOME") else {
        return path.to_path_buf();
    };
    if text == "~" {
        return PathBuf::from(home);
    }
    match text.strip_prefix("~/") {
        Some(rest) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

pub fn default_config_path() -> PathBuf {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("boswell").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_in_around_a_bare_repo_entry() {
        let config: Config = toml::from_str("[[repo]]\npath = \"/tmp/x\"\n").unwrap();
        let repo = &config.repos[0];
        assert_eq!(repo.remote, "origin");
        assert!(repo.pull);
        assert_eq!(repo.debounce, Duration::from_secs(5));
        assert_eq!(repo.recheck, Duration::from_secs(600));
        assert_eq!(config.retry.attempts, 6);
        assert_eq!(config.retry.base, Duration::from_secs(5));
        assert_eq!(config.retry.max, Duration::from_secs(300));
        assert_eq!(config.github.api_url, "https://api.github.com");
    }

    #[test]
    fn durations_are_read_as_human_time() {
        let config: Config = toml::from_str(
            "[retry]\nbase = \"250ms\"\n[[repo]]\npath = \"/tmp/x\"\ndebounce = \"1m\"\n",
        )
        .unwrap();
        assert_eq!(config.retry.base, Duration::from_millis(250));
        assert_eq!(config.repos[0].debounce, Duration::from_secs(60));
    }

    #[test]
    fn an_empty_config_is_rejected() {
        let mut config: Config = toml::from_str("").unwrap();
        assert!(config.prepare().is_err());
    }

    #[test]
    fn tilde_expands_only_at_the_front() {
        let home = PathBuf::from(env::var_os("HOME").expect("HOME"));
        assert_eq!(expand_tilde(Path::new("~/docs")), home.join("docs"));
        assert_eq!(expand_tilde(Path::new("/a/~/b")), Path::new("/a/~/b"));
    }
}
