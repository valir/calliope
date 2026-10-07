//! HTTP side: routing, validation of uploads and the replies of API v1.

use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;
use std::time::Instant;

use calliope_lib::stems_api::{
    flac_info, is_valid_job_id, is_valid_stem_name, ErrorBody, FlacError, Health, JobCreated, JobState,
    API_VERSION, SERVICE_NAME,
};
use tiny_http::{Header, Request, Response, StatusCode};

use crate::cli::VERSION;
use crate::config::Config;
use crate::jobs::{Manager, StemLookup};
use crate::{log, workdir};

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Health,
    CreateJob,
    JobStatus(String),
    Stem(String, String),
    DeleteJob(String),
    NotFound,
    MethodNotAllowed,
}

/// Maps a method and a path (without the query) to a route. Invalid ids and stem names are
/// unknown paths.
pub fn route(method: &str, path: &str) -> Route {
    let allow = |want: &str, route: Route| if method == want { route } else { Route::MethodNotAllowed };
    let parts: Vec<&str> = path.split('/').collect();
    // The path starts with "/", so the first part is empty.
    match parts.as_slice() {
        ["", "v1", "health"] => allow("GET", Route::Health),
        ["", "v1", "jobs"] => allow("POST", Route::CreateJob),
        ["", "v1", "jobs", id] if is_valid_job_id(id) => match method {
            "GET" => Route::JobStatus(id.to_string()),
            "DELETE" => Route::DeleteJob(id.to_string()),
            _ => Route::MethodNotAllowed,
        },
        ["", "v1", "jobs", id, "stems", name] if is_valid_job_id(id) && is_valid_stem_name(name) => {
            allow("GET", Route::Stem(id.to_string(), name.to_string()))
        }
        _ => Route::NotFound,
    }
}

enum Body {
    Bytes(Vec<u8>),
    File(File, u64),
    Empty,
}

pub struct Reply {
    status: u16,
    body: Body,
    content_type: &'static str,
    /// The request body was not (fully) read: do not reuse the connection.
    close: bool,
}

impl Reply {
    fn json<T: serde::Serialize>(status: u16, value: &T) -> Reply {
        Reply {
            status,
            body: Body::Bytes(serde_json::to_vec(value).expect("serialisable")),
            content_type: "application/json",
            close: false,
        }
    }

    fn error(status: u16, msg: &str) -> Reply {
        Reply::json(status, &ErrorBody { error: msg.to_string() })
    }

    fn closing(mut self) -> Reply {
        self.close = true;
        self
    }

    fn len(&self) -> u64 {
        match &self.body {
            Body::Bytes(b) => b.len() as u64,
            Body::File(_, n) => *n,
            Body::Empty => 0,
        }
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn header_value(req: &Request, name: &str) -> Option<String> {
    req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
}

/// Handles one request on its own thread and logs it.
pub fn handle(mgr: &Arc<Manager>, cfg: &Config, mut req: Request) {
    let start = Instant::now();
    let method = req.method().as_str().to_string();
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let peer = req.remote_addr().map(|a| a.to_string()).unwrap_or_else(|| "-".into());

    let reply = match route(&method, path) {
        Route::Health => Reply::json(
            200,
            &Health {
                service: SERVICE_NAME.into(),
                api: API_VERSION,
                version: VERSION.into(),
                models: vec![cfg.model.clone()],
                default_model: cfg.model.clone(),
                busy: mgr.busy(),
                max_duration_s: cfg.max_duration_s,
                max_upload_bytes: cfg.max_upload_bytes,
            },
        ),
        Route::CreateJob => create_job(mgr, cfg, &mut req, query),
        Route::JobStatus(id) => match mgr.status(&id) {
            Some(s) => Reply::json(200, &s),
            None => Reply::error(404, "unknown job"),
        },
        Route::Stem(id, name) => match mgr.stem_path(&id, &name) {
            StemLookup::Found(path) => match File::open(&path).and_then(|f| f.metadata().map(|m| (f, m.len()))) {
                Ok((f, n)) => Reply { status: 200, body: Body::File(f, n), content_type: "audio/flac", close: false },
                Err(_) => Reply::error(404, "stem file is gone"),
            },
            StemLookup::UnknownJob | StemLookup::UnknownStem => Reply::error(404, "unknown stem"),
            StemLookup::NotDone => Reply::error(409, "the job is not done"),
        },
        Route::DeleteJob(id) => {
            if mgr.delete(&id) {
                Reply { status: 204, body: Body::Empty, content_type: "application/json", close: false }
            } else {
                Reply::error(404, "unknown job")
            }
        }
        Route::NotFound => Reply::error(404, "unknown path"),
        Route::MethodNotAllowed => Reply::error(405, "method not allowed"),
    };
    // A POST that was answered without reading its body must not leave it in the connection.
    let reply = if method == "POST" && reply.status >= 400 { reply.closing() } else { reply };

    let (status, bytes) = (reply.status, reply.len());
    let mut headers = vec![header("Content-Type", reply.content_type)];
    if reply.close {
        headers.push(header("Connection", "close"));
    }
    let (reader, len): (Box<dyn Read + Send>, usize) = match reply.body {
        Body::Bytes(b) => {
            let n = b.len();
            (Box::new(Cursor::new(b)), n)
        }
        Body::File(f, n) => (Box::new(f), n as usize),
        Body::Empty => (Box::new(std::io::empty()), 0),
    };
    let _ = req.respond(Response::new(StatusCode(status), headers, reader, Some(len), None));
    log(format_args!(
        "request method={method} path={path} status={status} bytes={bytes} ms={} peer={peer}",
        start.elapsed().as_millis()
    ));
}

fn is_flac_content_type(value: &str) -> bool {
    let essence = value.split(';').next().unwrap_or("").trim();
    essence.eq_ignore_ascii_case("audio/flac") || essence.eq_ignore_ascii_case("audio/x-flac")
}

fn create_job(mgr: &Arc<Manager>, cfg: &Config, req: &mut Request, query: &str) -> Reply {
    let Some(length) = req.body_length() else {
        return Reply::error(411, "Content-Length is required");
    };
    let length = length as u64;
    if !header_value(req, "Content-Type").is_some_and(|v| is_flac_content_type(&v)) {
        return Reply::error(415, "Content-Type must be audio/flac");
    }
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == "model" && v != cfg.model {
            return Reply::error(400, "unknown model");
        }
    }
    if length > cfg.max_upload_bytes {
        return Reply::error(413, "the upload is too large");
    }
    if length == 0 {
        return Reply::error(415, "not a FLAC file");
    }
    let Some(slot) = mgr.reserve() else {
        return Reply::error(503, "the queue is full; try again later");
    };

    let id = uuid::Uuid::new_v4().to_string();
    let dir = match workdir::create_job_dir(&cfg.work_dir, &id) {
        Ok(d) => d,
        Err(e) => {
            log(format_args!("job id={id} error=\"cannot create the job folder: {e}\""));
            return Reply::error(500, "cannot create the job folder");
        }
    };
    match receive(cfg, req, length, &dir) {
        Ok(()) => {
            mgr.enqueue(slot, id.clone(), dir);
            Reply::json(202, &JobCreated { job: id, state: JobState::Queued })
        }
        Err((status, msg)) => {
            drop(slot);
            let _ = workdir::remove_job_dir(&cfg.work_dir, &id);
            log(format_args!("job id={id} state=refused error=\"{msg}\""));
            Reply::error(status, &msg)
        }
    }
}

/// Streams the body to `input.flac.part`, checking the header first and the length last, then
/// renames it to `input.flac`.
fn receive(cfg: &Config, req: &mut Request, length: u64, dir: &std::path::Path) -> Result<(), (u16, String)> {
    let io_err = |e: std::io::Error| (500u16, format!("cannot store the upload: {e}"));
    let part = dir.join("input.flac.part");
    let mut reader = req.as_reader().take(length);

    // The FLAC magic and STREAMINFO come first: refuse a wrong file before storing megabytes.
    let mut head = Vec::with_capacity(64);
    reader.by_ref().take(42).read_to_end(&mut head).map_err(|_| (400u16, "the upload was interrupted".to_string()))?;
    match flac_info(&mut Cursor::new(&head)) {
        Ok(info) if info.duration_s > cfg.max_duration_s as f64 => {
            return Err((
                413,
                format!("the audio is longer than the limit of {} minutes", cfg.max_duration_s.div_ceil(60)),
            ))
        }
        Ok(_) => {}
        Err(FlacError::Io(_)) => return Err((400, "the upload was interrupted".into())),
        Err(e) => return Err((415, e.to_string())),
    }

    let mut file = File::create(&part).map_err(io_err)?;
    file.write_all(&head).map_err(io_err)?;
    let mut total = head.len() as u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|_| (400u16, "the upload was interrupted".to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(io_err)?;
        total += n as u64;
    }
    if total != length {
        return Err((400, "the upload ended before Content-Length bytes arrived".into()));
    }
    file.sync_all().map_err(io_err)?;
    std::fs::rename(&part, dir.join("input.flac")).map_err(io_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";

    #[test]
    fn route_table() {
        assert_eq!(route("GET", "/v1/health"), Route::Health);
        assert_eq!(route("POST", "/v1/jobs"), Route::CreateJob);
        assert_eq!(route("GET", &format!("/v1/jobs/{ID}")), Route::JobStatus(ID.into()));
        assert_eq!(route("DELETE", &format!("/v1/jobs/{ID}")), Route::DeleteJob(ID.into()));
        assert_eq!(route("GET", &format!("/v1/jobs/{ID}/stems/vocals")), Route::Stem(ID.into(), "vocals".into()));
        // wrong methods
        let job = format!("/v1/jobs/{ID}");
        let stem = format!("/v1/jobs/{ID}/stems/vocals");
        for (m, p) in [
            ("POST", "/v1/health"),
            ("DELETE", "/v1/health"),
            ("GET", "/v1/jobs"),
            ("PUT", "/v1/jobs"),
            ("POST", job.as_str()),
            ("PUT", job.as_str()),
            ("POST", stem.as_str()),
            ("DELETE", stem.as_str()),
            ("HEAD", "/v1/health"),
        ] {
            assert_eq!(route(m, p), Route::MethodNotAllowed, "{m} {p}");
        }
        // unknown paths
        let unknown: Vec<String> = [
            "/", "/v1", "/v1/", "/health", "/v2/health", "/v1/health/", "/v1/jobs/", "/v1/jobs/../health",
            "/v1/jobs/UPPER", "/v1/jobs/a%2Fb",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([
            format!("/v1/jobs/{ID}/stems"),
            format!("/v1/jobs/{ID}/stems/"),
            format!("/v1/jobs/{ID}/stems/..%2Fx"),
            format!("/v1/jobs/{ID}/stems/Vocals.flac"),
            format!("/v1/jobs/{ID}/other"),
            format!("/v1/jobs/{ID}/stems/a/b"),
        ])
        .collect();
        for p in &unknown {
            assert_eq!(route("GET", p), Route::NotFound, "{p}");
        }
    }

    #[test]
    fn content_types() {
        assert!(is_flac_content_type("audio/flac"));
        assert!(is_flac_content_type("Audio/FLAC; charset=x"));
        assert!(is_flac_content_type("audio/x-flac"));
        assert!(!is_flac_content_type("audio/mpeg"));
        assert!(!is_flac_content_type("application/octet-stream"));
        assert!(!is_flac_content_type(""));
    }
}
