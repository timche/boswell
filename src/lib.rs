pub mod config;
pub mod daemon;
pub mod git;
pub mod issue;
pub mod subject;
pub mod sync;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
