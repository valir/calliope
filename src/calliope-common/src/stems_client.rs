//! HTTP client of "calliope-stems API v1" (feature `client`). Blocking; plain `http://` only
//! (the crate depends on `ureq` without TLS). The rules are those of plan section 2.7.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::stems_api::{
    is_valid_job_id, is_valid_stem_name, ErrorBody, Health, JobCreated, JobState, JobStatus, API_VERSION,
    MAX_STEMS, SERVICE_NAME,
};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound for one whole upload (the plan's "30 s without progress" cannot be enforced on a
/// blocked socket write; the cancel flag is checked between chunks).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// No state change for this long means the server is gone.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// A stem is fetched within this time (ureq has no per-read timeout; cancel is checked per chunk).
pub const STEM_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Consecutive failed polls tolerated before the server counts as gone.
pub const POLL_RETRIES: u32 = 3;
/// Server error text is cut to this many characters (same bound as yt-dlp / ffmpeg messages).
pub const MAX_ERROR_CHARS: usize = 300;
pub const MAX_JSON_BYTES: u64 = 64 * 1024;
pub const MAX_STEM_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// Connection refused, timeout, DNS failure.
    Unreachable(String),
    /// Reachable, but not a calliope-stems API v1 server (or an unusable answer).
    Incompatible(String),
    /// The server answered with an error status.
    Rejected { status: u16, message: String },
    /// A job is unknown to the server (it was restarted).
    Restarted,
    /// No state change for `STALL_TIMEOUT`.
    Stalled,
    Cancelled,
    /// A name, id or file broke a rule (client side or in the server's answer).
    Invalid(String),
    /// Local file problem.
    Io(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Unreachable(e) => write!(f, "Cannot reach the edge-AI server: {e}"),
            ClientError::Incompatible(e) => write!(f, "The edge-AI server is incompatible: {e}"),
            ClientError::Rejected { status, message } => write!(f, "The edge-AI server said {status}: {message}"),
            ClientError::Restarted => write!(f, "The edge-AI server restarted; try again"),
            ClientError::Stalled => write!(f, "The edge-AI server stopped responding"),
            ClientError::Cancelled => write!(f, "Cancelled"),
            ClientError::Invalid(e) => write!(f, "Invalid data from the edge-AI server: {e}"),
            ClientError::Io(e) => write!(f, "File error: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

fn net_error(e: ureq::Error) -> ClientError {
    match e {
        ureq::Error::Io(ref io) if io.kind() == std::io::ErrorKind::Interrupted => ClientError::Cancelled,
        other => ClientError::Unreachable(other.to_string()),
    }
}

/// The model name goes into a query string: only plain names (same rule as the server's flag).
fn valid_model(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 64
        && !m.starts_with('.')
        && m.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

pub struct StemsClient {
    base: String,
    agent: ureq::Agent,
    upload_agent: ureq::Agent,
}

fn agent(send_body: Option<Duration>, global: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(global)
        .timeout_send_body(send_body)
        .timeout_recv_response(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .build()
        .into()
}

/// Reads the upload and reports progress; stops when `cancel` is set.
struct CountingReader<'a, R, F: FnMut(u64, u64)> {
    inner: R,
    sent: u64,
    total: u64,
    cancel: &'a AtomicBool,
    on_sent: F,
}

impl<R: Read, F: FnMut(u64, u64)> Read for CountingReader<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.sent < self.total && self.cancel.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        let want = buf.len().min((self.total - self.sent) as usize);
        let n = self.inner.read(&mut buf[..want])?;
        self.sent += n as u64;
        if n > 0 {
            (self.on_sent)(self.sent, self.total);
        }
        Ok(n)
    }
}

impl StemsClient {
    /// `base` is the settings value, e.g. `http://archserver:8765` (a trailing `/` is ignored).
    pub fn new(base: &str) -> StemsClient {
        StemsClient {
            base: base.trim().trim_end_matches('/').to_string(),
            agent: agent(None, Some(REQUEST_TIMEOUT)),
            upload_agent: agent(Some(UPLOAD_TIMEOUT), None),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Turns an error status into `Rejected`, using the `{"error": ...}` body when there is one.
    fn check(mut resp: ureq::http::Response<ureq::Body>) -> Result<ureq::http::Response<ureq::Body>, ClientError> {
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(resp);
        }
        let message = resp
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()
            .ok()
            .and_then(|b| serde_json::from_slice::<ErrorBody>(&b).ok())
            .map(|e| e.error.chars().filter(|c| !c.is_control()).take(MAX_ERROR_CHARS).collect::<String>())
            .unwrap_or_else(|| format!("HTTP {status}"));
        Err(ClientError::Rejected { status, message })
    }

    fn json<T: serde::de::DeserializeOwned>(resp: ureq::http::Response<ureq::Body>) -> Result<T, ClientError> {
        let mut resp = Self::check(resp)?;
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()
            .map_err(|e| ClientError::Incompatible(format!("unreadable answer: {e}")))?;
        serde_json::from_slice(&bytes).map_err(|e| ClientError::Incompatible(format!("unexpected answer: {e}")))
    }

    /// `GET /v1/health`; the service must be calliope-stems API v1.
    pub fn health(&self) -> Result<Health, ClientError> {
        let resp = self.agent.get(&self.url("/v1/health")).call().map_err(net_error)?;
        let h: Health = Self::json(resp)?;
        if h.service != SERVICE_NAME {
            return Err(ClientError::Incompatible(format!("the service is '{}', not '{SERVICE_NAME}'", h.service)));
        }
        if h.api != API_VERSION {
            return Err(ClientError::Incompatible(format!("API version {} (this Calliope speaks {API_VERSION})", h.api)));
        }
        if !valid_model(&h.default_model) {
            return Err(ClientError::Incompatible("the default model name is not valid".into()));
        }
        Ok(h)
    }

    /// Uploads `file` (FLAC) for the server's default model and returns the job id.
    /// `on_sent(sent, total)` is called as bytes go out. When `cancel` becomes true the upload
    /// stops with `Cancelled`; if the server had already accepted the job it is deleted.
    pub fn submit(
        &self,
        file: &Path,
        cancel: &AtomicBool,
        on_sent: impl FnMut(u64, u64),
    ) -> Result<String, ClientError> {
        let health = self.health()?;
        let f = File::open(file).map_err(|e| ClientError::Io(format!("{}: {e}", file.display())))?;
        let total = f.metadata().map_err(|e| ClientError::Io(e.to_string()))?.len();
        if total > health.max_upload_bytes {
            return Err(ClientError::Invalid(format!(
                "the file is {total} bytes; the server accepts at most {}",
                health.max_upload_bytes
            )));
        }
        let mut reader = CountingReader { inner: f, sent: 0, total, cancel, on_sent };
        let url = format!("{}?model={}", self.url("/v1/jobs"), health.default_model);
        let result = self
            .upload_agent
            .post(&url)
            .header("Content-Type", "audio/flac")
            .header("Content-Length", total.to_string())
            .send(ureq::SendBody::from_reader(&mut reader));
        let resp = match result {
            Ok(r) => r,
            Err(_) if cancel.load(Ordering::SeqCst) => return Err(ClientError::Cancelled),
            Err(e) => return Err(net_error(e)),
        };
        let created: JobCreated = Self::json(resp)?;
        if !is_valid_job_id(&created.job) {
            return Err(ClientError::Invalid("the job id is not valid".into()));
        }
        if cancel.load(Ordering::SeqCst) {
            self.delete(&created.job);
            return Err(ClientError::Cancelled);
        }
        Ok(created.job)
    }

    /// `GET /v1/jobs/<id>`; an unknown job is `Restarted`.
    pub fn status(&self, job: &str) -> Result<JobStatus, ClientError> {
        if !is_valid_job_id(job) {
            return Err(ClientError::Invalid("bad job id".into()));
        }
        let resp = self.agent.get(&self.url(&format!("/v1/jobs/{job}"))).call().map_err(net_error)?;
        if resp.status().as_u16() == 404 {
            return Err(ClientError::Restarted);
        }
        let st: JobStatus = Self::json(resp)?;
        if st.job != job {
            return Err(ClientError::Invalid("the answer is for another job".into()));
        }
        if let Some(stems) = &st.stems {
            if stems.len() > MAX_STEMS || !stems.iter().all(|s| is_valid_stem_name(s)) {
                return Err(ClientError::Invalid("bad stem list".into()));
            }
        }
        Ok(st)
    }

    /// Polls every `interval` until the job is final. `on_status` sees every answer. Fails with
    /// `Stalled` when nothing changed for `stall`, `Cancelled` when `cancel` is set (the job is
    /// deleted on the server, best effort).
    pub fn wait_final(
        &self,
        job: &str,
        cancel: &AtomicBool,
        interval: Duration,
        stall: Duration,
        mut on_status: impl FnMut(&JobStatus),
    ) -> Result<JobStatus, ClientError> {
        let mut last: Option<(JobState, Option<u64>)> = None;
        let mut changed = Instant::now();
        let mut unreachable_polls = 0u32;
        loop {
            if cancel.load(Ordering::SeqCst) {
                self.delete(job);
                return Err(ClientError::Cancelled);
            }
            let st = match self.status(job) {
                Ok(st) => {
                    unreachable_polls = 0;
                    st
                }
                Err(ClientError::Unreachable(_)) if unreachable_polls < POLL_RETRIES => {
                    unreachable_polls += 1;
                    std::thread::sleep(interval);
                    continue;
                }
                Err(e) => return Err(e),
            };
            on_status(&st);
            if st.state.is_final() {
                return Ok(st);
            }
            let key = (st.state, st.progress.map(|p| (p * 1000.0) as u64));
            if last != Some(key) {
                last = Some(key);
                changed = Instant::now();
            } else if changed.elapsed() >= stall {
                return Err(ClientError::Stalled);
            }
            std::thread::sleep(interval);
        }
    }

    /// Downloads one stem into `dest_part` (replaced if present). Checks the `fLaC` magic and
    /// the size limit; stops with `Cancelled` when `cancel` is set; on any error the partial file is removed. Returns the size.
    pub fn fetch_stem(&self, job: &str, name: &str, dest_part: &Path, cancel: &AtomicBool) -> Result<u64, ClientError> {
        if !is_valid_job_id(job) || !is_valid_stem_name(name) {
            return Err(ClientError::Invalid("bad job id or stem name".into()));
        }
        let resp = self
            .agent
            .get(&self.url(&format!("/v1/jobs/{job}/stems/{name}")))
            .config()
            .timeout_global(None)
            .timeout_recv_body(Some(STEM_TIMEOUT))
            .build()
            .call()
            .map_err(net_error)?;
        if resp.status().as_u16() == 404 {
            return Err(ClientError::Restarted);
        }
        let mut resp = Self::check(resp)?;
        let io = |e: std::io::Error| ClientError::Io(format!("{}: {e}", dest_part.display()));
        let mut file = File::create(dest_part).map_err(io)?;
        let result = (|| {
            let mut body = resp.body_mut().with_config().limit(MAX_STEM_BYTES).reader();
            let mut head = [0u8; 4];
            body.read_exact(&mut head)
                .map_err(|_| ClientError::Invalid(format!("stem '{name}' is not a FLAC file")))?;
            if &head != b"fLaC" {
                return Err(ClientError::Invalid(format!("stem '{name}' is not a FLAC file")));
            }
            file.write_all(&head).map_err(io)?;
            let mut total = 4u64;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(ClientError::Cancelled);
                }
                let n = body.read(&mut buf).map_err(|e| ClientError::Unreachable(e.to_string()))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(io)?;
                total += n as u64;
            }
            file.sync_all().map_err(io)?;
            Ok(total)
        })();
        if result.is_err() {
            drop(file);
            let _ = std::fs::remove_file(dest_part);
        }
        result
    }

    /// `DELETE /v1/jobs/<id>`, best effort (errors are ignored: the server cleans up anyway).
    pub fn delete(&self, job: &str) {
        if is_valid_job_id(job) {
            let _ = self.agent.delete(&self.url(&format!("/v1/jobs/{job}"))).call();
        }
    }
}
