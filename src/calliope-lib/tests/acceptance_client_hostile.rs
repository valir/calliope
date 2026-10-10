//! QA probes of the stems client (`calliope_lib::stems_client`) against hostile or broken
//! "servers": scripted raw HTTP on 127.0.0.1 only. The client talks to whatever address the
//! user typed in Settings, so everything it gets back is untrusted.
#![cfg(feature = "client")]
#![allow(clippy::all)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use calliope_lib::stems_client::{ClientError, StemsClient};

type Handler = dyn Fn(&Request, &mut TcpStream) + Send + Sync + 'static;

struct Request {
    line: String,
    #[allow(dead_code)]
    headers: Vec<(String, String)>,
}

struct Fake {
    addr: SocketAddr,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

impl Fake {
    fn client(&self) -> StemsClient {
        StemsClient::new(&format!("http://{}", self.addr))
    }
}

fn serve(h: impl Fn(&Request, &mut TcpStream) + Send + Sync + 'static) -> Fake {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    let hits = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let h: Arc<Handler> = Arc::new(h);
    let (hits2, stop2) = (hits.clone(), stop.clone());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            if stop2.load(Ordering::SeqCst) {
                return;
            }
            let Ok(mut s) = conn else { continue };
            let (h, hits) = (h.clone(), hits2.clone());
            std::thread::spawn(move || {
                s.set_read_timeout(Some(Duration::from_secs(20))).ok();
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match s.read(&mut b) {
                        Ok(1) => head.push(b[0]),
                        _ => return,
                    }
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let mut lines = text.split("\r\n");
                let line = lines.next().unwrap_or("").to_string();
                let headers = lines
                    .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())))
                    .collect();
                hits.lock().unwrap().push(line.clone());
                h(&Request { line, headers }, &mut s);
            });
        }
    });
    Fake { addr, hits, stop }
}

fn reply(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn json(s: &mut TcpStream, status: &str, v: serde_json::Value) {
    reply(s, status, "application/json", v.to_string().as_bytes());
}

fn health_json() -> serde_json::Value {
    serde_json::json!({"service":"calliope-stems","api":1,"version":"1","models":["htdemucs_6s"],
        "default_model":"htdemucs_6s","busy":false,"max_duration_s":900,"max_upload_bytes":1_000_000})
}

fn tmpfile(content: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("audio.flac");
    std::fs::write(&p, content).unwrap();
    (d, p)
}

fn flac_bytes(n: usize) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    v.resize(n, 7);
    v
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn health_rejects_every_kind_of_wrong_answer() {
    type Case = (&'static str, Box<dyn Fn(&mut TcpStream) + Send + Sync>);
    let cases: Vec<Case> = vec![
        ("wrong service", Box::new(|s| { let mut h = health_json(); h["service"] = "evil".into(); json(s, "200 OK", h) })),
        ("api 2", Box::new(|s| { let mut h = health_json(); h["api"] = 2.into(); json(s, "200 OK", h) })),
        ("api 0", Box::new(|s| { let mut h = health_json(); h["api"] = 0.into(); json(s, "200 OK", h) })),
        ("model traversal", Box::new(|s| { let mut h = health_json(); h["default_model"] = "../../x".into(); json(s, "200 OK", h) })),
        ("model with ampersand", Box::new(|s| { let mut h = health_json(); h["default_model"] = "a&model=b".into(); json(s, "200 OK", h) })),
        ("model with space/newline", Box::new(|s| { let mut h = health_json(); h["default_model"] = "a b\r\nX: y".into(); json(s, "200 OK", h) })),
        ("model dot-start", Box::new(|s| { let mut h = health_json(); h["default_model"] = ".hidden".into(); json(s, "200 OK", h) })),
        ("model 65 chars", Box::new(|s| { let mut h = health_json(); h["default_model"] = "a".repeat(65).into(); json(s, "200 OK", h) })),
        ("html", Box::new(|s| reply(s, "200 OK", "text/html", b"<html>router login</html>"))),
        ("empty", Box::new(|s| reply(s, "200 OK", "application/json", b""))),
        ("json null", Box::new(|s| reply(s, "200 OK", "application/json", b"null"))),
        ("huge json", Box::new(|s| { let pad = "x".repeat(200_000); let mut h = health_json(); h["version"] = pad.into(); json(s, "200 OK", h) })),
        ("500", Box::new(|s| json(s, "500 Internal Server Error", serde_json::json!({"error":"boom"})))),
        ("redirect", Box::new(|s| { let _ = s.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/v1/health\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); })),
        ("garbage", Box::new(|s| { let _ = s.write_all(b"\x00\x01garbage\r\n\r\n"); })),
        ("closed", Box::new(|_s| {})),
    ];
    for (name, f) in cases {
        let fake = serve(move |_, s| f(s));
        let r = fake.client().health();
        assert!(r.is_err(), "{name}: accepted: {r:?}");
        if name == "redirect" {
            assert_eq!(fake.hits.lock().unwrap().len(), 1, "a redirect must not be followed");
        }
    }
}

#[test]
fn submit_validates_what_the_server_returns() {
    let bad_ids = ["../../etc", "..", "", &"a".repeat(65), "-rf", "A-B", "a/b", "a b", "%2e%2e"];
    for id in bad_ids {
        let id = id.to_string();
        let fake = serve(move |req, s| {
            if req.line.starts_with("GET /v1/health") {
                json(s, "200 OK", health_json());
            } else {
                // drain the body so the client sees our reply
                json(s, "202 Accepted", serde_json::json!({"job": id, "state":"queued"}));
            }
        });
        let (_d, f) = tmpfile(&flac_bytes(2000));
        let r = fake.client().submit(&f, &no_cancel(), |_, _| {});
        assert!(matches!(r, Err(ClientError::Invalid(_))), "bad job id accepted or wrong error: {r:?}");
    }
}

#[test]
fn submit_refuses_a_file_over_the_servers_limit_before_sending_it() {
    let fake = serve(|req, s| {
        assert!(!req.line.starts_with("POST"), "nothing may be uploaded");
        let mut h = health_json();
        h["max_upload_bytes"] = 1000.into();
        json(s, "200 OK", h)
    });
    let (_d, f) = tmpfile(&flac_bytes(5000));
    let r = fake.client().submit(&f, &no_cancel(), |_, _| {});
    assert!(matches!(r, Err(ClientError::Invalid(_))), "{r:?}");
}

#[test]
fn submit_surfaces_a_413_and_413_json_message() {
    let fake = serve(|req, s| {
        if req.line.starts_with("GET /v1/health") {
            json(s, "200 OK", health_json());
        } else {
            json(s, "413 Payload Too Large", serde_json::json!({"error":"the audio is longer than the limit of 15 minutes"}));
        }
    });
    let (_d, f) = tmpfile(&flac_bytes(2000));
    match fake.client().submit(&f, &no_cancel(), |_, _| {}) {
        Err(ClientError::Rejected { status: 413, message }) => assert!(message.contains("15 minutes")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn submit_missing_or_unreadable_file_is_an_io_error() {
    let fake = serve(|_, s| json(s, "200 OK", health_json()));
    let r = fake.client().submit(std::path::Path::new("/nonexistent-qa/x.flac"), &no_cancel(), |_, _| {});
    assert!(matches!(r, Err(ClientError::Io(_))), "{r:?}");
}

#[test]
fn status_validates_stems_and_job_binding() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let long: Vec<String> = (0..17).map(|i| format!("s{i}")).collect();
    let cases: Vec<(&str, serde_json::Value, bool)> = vec![
        ("ok", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["vocals","drums"],"error":null}), true),
        ("traversal", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["../../x"],"error":null}), false),
        ("dotdot", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":[".."],"error":null}), false),
        ("slash", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["a/b"],"error":null}), false),
        ("uppercase", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["Vocals"],"error":null}), false),
        ("with .flac", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["vocals.flac"],"error":null}), false),
        ("empty name", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":[""],"error":null}), false),
        ("long name", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["a".repeat(33)],"error":null}), false),
        ("17 stems", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":long,"error":null}), false),
        ("other job", serde_json::json!({"job":"0190b1c2-3d4e-4f50-8a6b-ffffffffffff","state":"done","progress":1.0,"stems":["vocals"],"error":null}), false),
        ("unknown state", serde_json::json!({"job":job,"state":"exploded","progress":1.0,"stems":null,"error":null}), false),
        ("progress string", serde_json::json!({"job":job,"state":"running","progress":"half","stems":null,"error":null}), false),
    ];
    for (name, body, ok) in cases {
        let fake = serve(move |_, s| json(s, "200 OK", body.clone()));
        let r = fake.client().status(job);
        assert_eq!(r.is_ok(), ok, "{name}: {r:?}");
    }
    // A job id the caller made up is refused without a request.
    let fake = serve(|_, s| json(s, "200 OK", serde_json::json!({})));
    assert!(matches!(fake.client().status("../x"), Err(ClientError::Invalid(_))));
    assert!(fake.hits.lock().unwrap().is_empty());
    // 404 -> restarted
    let fake = serve(|_, s| json(s, "404 Not Found", serde_json::json!({"error":"unknown job"})));
    assert!(matches!(fake.client().status(job), Err(ClientError::Restarted)));
}

#[test]
fn status_validates_stem_level_list() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let lv = |n: &str, ms: u64, db: i32, w: u32| serde_json::json!({"name":n,"audible_ms":ms,"level_dbfs":db,"window_ms":w});
    let with = |levels: serde_json::Value| {
        serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["vocals","piano"],"error":null,"stem_levels":levels})
    };
    let cases: Vec<(&str, serde_json::Value, bool)> = vec![
        ("ok", with(serde_json::json!([lv("vocals", 200600, -40, 100), lv("piano", 0, -40, 100)])), true),
        ("empty", with(serde_json::json!([])), true),
        ("unknown name", with(serde_json::json!([lv("drums", 0, -40, 100)])), false),
        ("duplicate", with(serde_json::json!([lv("piano", 0, -40, 100), lv("piano", 0, -40, 100)])), false),
        ("window 0", with(serde_json::json!([lv("piano", 0, -40, 0)])), false),
        ("window 1001", with(serde_json::json!([lv("piano", 0, -40, 1001)])), false),
        ("level above 0", with(serde_json::json!([lv("piano", 0, 1, 100)])), false),
        ("level below -150", with(serde_json::json!([lv("piano", 0, -151, 100)])), false),
        ("not a window multiple", with(serde_json::json!([lv("piano", 150, -40, 100)])), false),
        ("longer than a day", with(serde_json::json!([lv("piano", 86_400_100, -40, 100)])), false),
        ("negative audible_ms", with(serde_json::json!([{"name":"piano","audible_ms":-1,"level_dbfs":-40,"window_ms":100}])), false),
        ("audible_ms string", with(serde_json::json!([{"name":"piano","audible_ms":"0","level_dbfs":-40,"window_ms":100}])), false),
        ("not a list", with(serde_json::json!("lots")), false),
        ("17 entries", with(serde_json::json!((0..17).map(|_| lv("piano", 0, -40, 100)).collect::<Vec<_>>())), false),
        ("levels without stems", serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":null,"error":null,"stem_levels":[lv("piano", 0, -40, 100)]}), false),
    ];
    for (name, body, ok) in cases {
        let fake = serve(move |_, s| json(s, "200 OK", body.clone()));
        let r = fake.client().status(job);
        assert_eq!(r.is_ok(), ok, "{name}: {r:?}");
        // Type errors are an unusable answer; rule breaks are Invalid.
        let wrong_type = ["negative audible_ms", "audible_ms string", "not a list"].contains(&name);
        match r {
            Err(ClientError::Incompatible(_)) => assert!(wrong_type, "{name}"),
            Err(ClientError::Invalid(m)) => assert!(!wrong_type && m == "bad stem level list", "{name}: {m}"),
            _ => {}
        }
    }
    // No key: fine, and nothing measured.
    let body = serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["vocals"],"error":null});
    let fake = serve(move |_, s| json(s, "200 OK", body.clone()));
    assert_eq!(fake.client().status(job).unwrap().stem_levels, None);
}

#[test]
fn status_validates_stem_level_list_only_when_done() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let bad = serde_json::json!([{"name":"drums","audible_ms":0,"level_dbfs":-40,"window_ms":100}]);
    let body = serde_json::json!({"job":job,"state":"done","progress":1.0,"stems":["vocals"],"error":null,"stem_levels":bad});
    let fake = serve(move |_, s| json(s, "200 OK", body.clone()));
    match fake.client().status(job) {
        Err(ClientError::Invalid(m)) => assert_eq!(m, "bad stem level list"),
        other => panic!("{other:?}"),
    }
    let body = serde_json::json!({"job":job,"state":"failed","progress":null,"stems":null,"error":"boom",
        "stem_levels":bad});
    let fake = serve(move |_, s| json(s, "200 OK", body.clone()));
    let st = fake.client().status(job).unwrap();
    assert_eq!(st.error.as_deref(), Some("boom"));
    assert_eq!(st.stem_levels, None);
}

#[test]
fn fetch_stem_refuses_non_flac_and_cleans_up() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let cases: Vec<(&str, Box<dyn Fn(&mut TcpStream) + Send + Sync>)> = vec![
        ("html", Box::new(|s| reply(s, "200 OK", "audio/flac", b"<html>hello</html>"))),
        ("empty", Box::new(|s| reply(s, "200 OK", "audio/flac", b""))),
        ("3 bytes", Box::new(|s| reply(s, "200 OK", "audio/flac", b"fLa"))),
        ("lying length", Box::new(|s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: 9999999999\r\nConnection: close\r\n\r\nfLaCabcdef");
        })),
        ("409", Box::new(|s| json(s, "409 Conflict", serde_json::json!({"error":"not done"})))),
        ("500", Box::new(|s| reply(s, "500 Internal Server Error", "text/plain", b"x"))),
    ];
    for (name, f) in cases {
        let fake = serve(move |_, s| f(s));
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("vocals.flac.part");
        let r = fake.client().fetch_stem(job, "vocals", &dest, &AtomicBool::new(false));
        assert!(r.is_err(), "{name}: accepted {r:?}");
        assert!(!dest.exists(), "{name}: partial file left behind");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0, "{name}");
    }
    // 404 is "restarted"
    let fake = serve(|_, s| json(s, "404 Not Found", serde_json::json!({"error":"x"})));
    let d = tempfile::tempdir().unwrap();
    assert!(matches!(fake.client().fetch_stem(job, "vocals", &d.path().join("p"), &AtomicBool::new(false)), Err(ClientError::Restarted)));
    // names are validated before any request
    for (j, n) in [("../x", "vocals"), (job, "../x"), (job, "A"), (job, "")] {
        assert!(matches!(fake.client().fetch_stem(j, n, &d.path().join("p"), &AtomicBool::new(false)), Err(ClientError::Invalid(_))), "{j} {n}");
    }
    assert!(!d.path().join("p").exists());
}

#[test]
fn fetch_stem_good_flac_is_stored_verbatim() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let body = flac_bytes(300_000);
    let b2 = body.clone();
    let fake = serve(move |req, s| {
        assert!(req.line.starts_with(&format!("GET /v1/jobs/{job}/stems/drums ")), "{}", req.line);
        reply(s, "200 OK", "audio/flac", &b2)
    });
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("drums.flac.part");
    let n = fake.client().fetch_stem(job, "drums", &dest, &AtomicBool::new(false)).unwrap();
    assert_eq!(n as usize, body.len());
    assert_eq!(std::fs::read(&dest).unwrap(), body);
}

#[test]
fn fetch_stem_honours_the_cancel_flag_and_removes_the_part() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let body = flac_bytes(300_000);
    let fake = serve(move |_, s| reply(s, "200 OK", "audio/flac", &body));
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("drums.flac.part");
    let r = fake.client().fetch_stem(job, "drums", &dest, &AtomicBool::new(true));
    assert_eq!(r, Err(ClientError::Cancelled));
    assert!(!dest.exists());
}

#[test]
fn wait_final_tolerates_a_few_unreachable_polls_but_not_forever() {
    // a closed port: every poll is Unreachable; the error comes after the retries, not at once
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let c = StemsClient::new(&format!("http://{addr}"));
    let t = std::time::Instant::now();
    let r = c.wait_final(
        "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b",
        &AtomicBool::new(false),
        Duration::from_millis(100),
        Duration::from_secs(60),
        |_| {},
    );
    assert!(matches!(r, Err(ClientError::Unreachable(_))), "{r:?}");
    assert!(t.elapsed() >= Duration::from_millis(300), "gave up after {:?}", t.elapsed());
}

#[test]
fn a_server_that_never_answers_times_out_instead_of_hanging_forever() {
    // Accepts and says nothing. The client must give up (REQUEST_TIMEOUT = 30 s); we only check
    // that health() returns an error within 45 s.
    let fake = serve(|_, _s| std::thread::sleep(Duration::from_secs(40)));
    let t = std::time::Instant::now();
    let r = fake.client().health();
    assert!(r.is_err());
    assert!(t.elapsed() < Duration::from_secs(45), "took {:?}", t.elapsed());
}

#[test]
fn connection_refused_is_unreachable_and_fast() {
    // Find a closed port.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let t = std::time::Instant::now();
    let r = StemsClient::new(&format!("http://{addr}")).health();
    assert!(matches!(r, Err(ClientError::Unreachable(_))), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn proxy_environment_is_ignored() {
    // A poisoned HTTP proxy must not be used (the edge-AI server is on the LAN).
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    std::env::set_var("HTTP_PROXY", format!("http://{dead}"));
    std::env::set_var("http_proxy", format!("http://{dead}"));
    std::env::set_var("ALL_PROXY", format!("http://{dead}"));
    let fake = serve(|_, s| json(s, "200 OK", health_json()));
    let r = fake.client().health();
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("http_proxy");
    std::env::remove_var("ALL_PROXY");
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn trailing_slash_and_spaces_in_the_base_url_are_tolerated() {
    let fake = serve(|req, s| {
        assert!(req.line.starts_with("GET /v1/health "), "{}", req.line);
        json(s, "200 OK", health_json())
    });
    for base in [format!("http://{}/", fake.addr), format!("  http://{}  ", fake.addr)] {
        assert!(StemsClient::new(&base).health().is_ok(), "{base:?}");
    }
}

#[test]
fn cancel_during_upload_stops_promptly_and_deletes_an_accepted_job() {
    let uploaded = Arc::new(AtomicUsize::new(0));
    let u2 = uploaded.clone();
    let deleted = Arc::new(AtomicBool::new(false));
    let d2 = deleted.clone();
    let fake = serve(move |req, s| {
        if req.line.starts_with("GET /v1/health") {
            let mut h = health_json();
            h["max_upload_bytes"] = 100_000_000u64.into();
            json(s, "200 OK", h)
        } else if req.line.starts_with("POST") {
            // read slowly
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                u2.fetch_add(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
            }
        } else if req.line.starts_with("DELETE") {
            d2.store(true, Ordering::SeqCst);
            let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
        }
    });
    let (_d, f) = tmpfile(&flac_bytes(8_000_000));
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let client = fake.client();
    let h = std::thread::spawn(move || client.submit(&f, &c2, |_, _| {}));
    std::thread::sleep(Duration::from_millis(500));
    let t = std::time::Instant::now();
    cancel.store(true, Ordering::SeqCst);
    let r = h.join().unwrap();
    assert!(matches!(r, Err(ClientError::Cancelled)), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(5), "cancel took {:?}", t.elapsed());
    assert!(uploaded.load(Ordering::SeqCst) < 8_000_000);
}

// ---------------------------------------------------------------- fix round 1 probes

/// A tiny one-shot-per-connection status server bound to a given address (for restart tests).
fn status_server_on(addr: SocketAddr, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let l = TcpListener::bind(addr).unwrap();
    l.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match l.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).ok();
                    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
                    let mut head = Vec::new();
                    let mut b = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match s.read(&mut b) {
                            Ok(1) => head.push(b[0]),
                            _ => break,
                        }
                    }
                    json(
                        &mut s,
                        "200 OK",
                        serde_json::json!({"job":"0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b","state":"done","progress":1.0,"stems":["vocals"],"error":null}),
                    );
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    })
}

#[test]
fn wait_final_survives_a_brief_outage_but_not_a_long_one() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    // reserve a port, keep it closed for ~700 ms (interval 300 ms, 3 retries = ~900 ms window), then serve
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    drop(l);
    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, a2) = (stop.clone(), addr);
    let starter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        status_server_on(a2, stop2).join().ok();
    });
    let c = StemsClient::new(&format!("http://{addr}"));
    let r = c.wait_final(job, &AtomicBool::new(false), Duration::from_millis(300), Duration::from_secs(60), |_| {});
    assert!(r.is_ok(), "brief outage must be survived: {r:?}");
    stop.store(true, Ordering::SeqCst);
    starter.join().unwrap();

    // a long outage: the server only comes back after 5 s; the client has long given up
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, a2) = (stop.clone(), addr);
    let starter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        status_server_on(a2, stop2).join().ok();
    });
    let t = std::time::Instant::now();
    let r = StemsClient::new(&format!("http://{addr}")).wait_final(job, &AtomicBool::new(false), Duration::from_millis(300), Duration::from_secs(60), |_| {});
    assert!(matches!(r, Err(ClientError::Unreachable(_))), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(4), "took {:?}", t.elapsed());
    stop.store(true, Ordering::SeqCst);
    starter.join().unwrap();
}

#[test]
fn cancel_during_a_slow_stem_stops_quickly_and_removes_the_part() {
    let job = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b";
    let fake = serve(|_, s| {
        let body = flac_bytes(2_000_000);
        let h = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = s.write_all(h.as_bytes());
        for chunk in body.chunks(4096) {
            if s.write_all(chunk).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50)); // ~40 s for the whole body
        }
    });
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("drums.flac.part");
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        c2.store(true, Ordering::SeqCst);
    });
    let t = std::time::Instant::now();
    let r = fake.client().fetch_stem(job, "drums", &dest, &cancel);
    assert_eq!(r, Err(ClientError::Cancelled));
    assert!(t.elapsed() < Duration::from_secs(3), "took {:?}", t.elapsed());
    assert!(!dest.exists(), "part left behind");
}
