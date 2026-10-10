//! The server's settings, filled by `cli`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_LISTEN: &str = "0.0.0.0:8765";
pub const DEFAULT_MODEL: &str = "htdemucs_6s";
pub const DEFAULT_MAX_UPLOAD_MB: u64 = 300;
pub const DEFAULT_QUEUE: usize = 2;
pub const DEFAULT_SEPARATOR_TIMEOUT_MIN: u64 = 30;
pub const DEFAULT_RETENTION_HOURS: u64 = 24;
pub const DEFAULT_JANITOR_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// Absolute.
    pub work_dir: PathBuf,
    /// Absolute path of the separator executable.
    pub separator: PathBuf,
    pub model: String,
    pub max_upload_bytes: u64,
    pub max_duration_s: u64,
    /// Jobs allowed to wait behind the running one.
    pub queue: usize,
    pub separator_timeout: Duration,
    pub retention: Duration,
    pub janitor_interval: Duration,
    /// Measure every stem's audible time and report it (`--no-stem-levels` turns it off).
    pub stem_levels: bool,
}
