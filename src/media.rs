//! Media handling for the stem import (plan section 2.6): `ffprobe` metadata, the mapping of
//! file tags to track fields, conversion to FLAC (44.1 kHz stereo) with `ffmpeg`, and the
//! 15-minute limit. Pure logic, no Tauri. Tools are started through `calliope_common::process`
//! (argv only, never a shell); sources are only read, as `file:<absolute path>` inputs, and
//! all output goes into the job folder given by the caller.
#![allow(dead_code)] // used by the import job (a later task) and the tests

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use calliope_common::process::{self, CancelHandle};

use crate::track_meta::TrackEdits;

/// Longest track accepted (owner decision Q4).
pub const MAX_DURATION_S: f64 = 15.0 * 60.0;
/// Largest source file accepted.
pub const MAX_FILE_BYTES: u64 = 1 << 30;
const MAX_PROBE_BYTES: usize = 4 << 20;
const MAX_ERROR_CHARS: usize = 300;

const MAX_NAME: usize = 200;
const MAX_COMPOSERS: usize = 20;
const MAX_COPYRIGHT: usize = 500;

pub const TOO_LONG_MESSAGE: &str = "Tracks longer than 15 minutes are not supported";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    Cancelled,
    Failed(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Cancelled => f.write_str("Cancelled"),
            MediaError::Failed(m) => f.write_str(m),
        }
    }
}

fn failed<T>(m: impl Into<String>) -> Result<T, MediaError> {
    Err(MediaError::Failed(m.into()))
}

/// A cancel switch shared between the job thread (which runs the tool) and whoever may stop
/// it. Cloning shares the state. Once cancelled, a tool started later is stopped at once.
#[derive(Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    current: Arc<Mutex<Option<CancelHandle>>>,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(h) = self.current.lock().ok().and_then(|g| g.clone()) {
            h.cancel();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    fn register(&self, h: CancelHandle) {
        if let Ok(mut g) = self.current.lock() {
            *g = Some(h.clone());
        }
        if self.is_cancelled() {
            h.cancel();
        }
    }

    fn clear(&self) {
        if let Ok(mut g) = self.current.lock() {
            *g = None;
        }
    }
}

/// What `ffprobe` told us about a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub duration_s: Option<f64>,
    pub has_audio: bool,
    pub has_video: bool,
    /// Lower-case tag names. Container tags win; tags of audio streams fill the gaps
    /// (Ogg and FLAC keep them there).
    pub tags: BTreeMap<String, String>,
}

struct Output {
    success: bool,
    stdout: String,
    stderr_last: String,
}

/// Runs a tool to the end, collecting stdout (capped) and the last non-empty stderr line.
fn run(program: &Path, args: &[String], cancel: &Cancel) -> Result<Output, MediaError> {
    let out = Arc::new(Mutex::new(String::new()));
    let err = Arc::new(Mutex::new(String::new()));
    let (o, e) = (out.clone(), err.clone());
    let running = process::spawn(
        program,
        args,
        None,
        move |l| {
            if let Ok(mut s) = o.lock() {
                if s.len() < MAX_PROBE_BYTES {
                    s.push_str(&l);
                    s.push('\n');
                }
            }
        },
        move |l| {
            if !l.trim().is_empty() {
                if let Ok(mut s) = e.lock() {
                    *s = l;
                }
            }
        },
    )
    .map_err(|e| MediaError::Failed(e.to_string()))?;
    cancel.register(running.cancel_handle());
    let status = running.wait();
    cancel.clear();
    let status = status.map_err(|e| MediaError::Failed(e.to_string()))?;
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let stdout = out.lock().map(|s| s.clone()).unwrap_or_default();
    let stderr_last = err.lock().map(|s| s.clone()).unwrap_or_default();
    Ok(Output { success: status.success(), stdout, stderr_last })
}

fn bare_name(file: &Path) -> String {
    file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.to_string_lossy().into_owned())
}

fn file_arg(file: &Path) -> String {
    format!("file:{}", file.display())
}

fn require_absolute(file: &Path) -> Result<(), MediaError> {
    if file.is_absolute() {
        Ok(())
    } else {
        failed(format!("internal error: {} is not an absolute path", file.display()))
    }
}

/// `ffprobe -v error -print_format json -show_format -show_streams file:<abs>`.
/// Errors: "Selected file <file> could not be read as audio or video", "... has no audio
/// track", "... is larger than 1 GiB". The duration limit is checked by [`check_duration`]
/// (and again by [`to_flac`]).
pub fn probe(ffprobe: &Path, file: &Path, cancel: &Cancel) -> Result<Probe, MediaError> {
    require_absolute(file)?;
    let name = bare_name(file);
    let unreadable = || MediaError::Failed(format!("Selected file {name} could not be read as audio or video"));
    match std::fs::metadata(file) {
        Ok(m) if m.is_file() => {
            if m.len() > MAX_FILE_BYTES {
                return failed(format!("Selected file {name} is larger than 1 GiB"));
            }
        }
        _ => return Err(unreadable()),
    }
    let args: Vec<String> =
        ["-v", "error", "-print_format", "json", "-show_format", "-show_streams"].iter().map(|s| s.to_string()).chain([file_arg(file)]).collect();
    let out = run(ffprobe, &args, cancel)?;
    if !out.success {
        return Err(unreadable());
    }
    let p = parse_probe(&out.stdout).ok_or_else(unreadable)?;
    if !p.has_audio {
        return failed(format!("Selected file {name} has no audio track"));
    }
    Ok(p)
}

/// Reads ffprobe's JSON. `None` if it isn't the expected shape.
pub fn parse_probe(json: &str) -> Option<Probe> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let streams = v.get("streams")?.as_array()?;
    let kind = |s: &serde_json::Value| s.get("codec_type").and_then(|c| c.as_str()).map(str::to_string);
    let has_audio = streams.iter().any(|s| kind(s).as_deref() == Some("audio"));
    // a cover picture is a "video" stream but not a video
    let has_video = streams.iter().any(|s| {
        kind(s).as_deref() == Some("video")
            && s.pointer("/disposition/attached_pic").and_then(|d| d.as_i64()) != Some(1)
    });
    let number = |x: Option<&serde_json::Value>| -> Option<f64> {
        let n = match x? {
            serde_json::Value::String(s) => s.parse::<f64>().ok()?,
            serde_json::Value::Number(n) => n.as_f64()?,
            _ => return None,
        };
        (n.is_finite() && n >= 0.0).then_some(n)
    };
    let duration_s = number(v.pointer("/format/duration"))
        .or_else(|| streams.iter().filter(|s| kind(s).as_deref() == Some("audio")).find_map(|s| number(s.get("duration"))));
    let mut tags = BTreeMap::new();
    let mut add = |obj: Option<&serde_json::Value>| {
        if let Some(m) = obj.and_then(|t| t.as_object()) {
            for (k, val) in m {
                if let Some(s) = val.as_str() {
                    tags.entry(k.to_lowercase()).or_insert_with(|| s.to_string());
                }
            }
        }
    };
    add(v.pointer("/format/tags"));
    for s in streams.iter().filter(|s| kind(s).as_deref() == Some("audio")) {
        add(s.get("tags"));
    }
    Some(Probe { duration_s, has_audio, has_video, tags })
}

/// Refuses sources longer than [`MAX_DURATION_S`] (and ones whose length is unknown).
pub fn check_duration(p: &Probe, file_name: &str) -> Result<(), MediaError> {
    match p.duration_s {
        Some(d) if d > MAX_DURATION_S => failed(TOO_LONG_MESSAGE),
        Some(_) => Ok(()),
        None => failed(format!("Selected file {file_name} could not be read as audio or video")),
    }
}

// ---- tags -> fields ----

/// Trims, drops control characters (a tab or newline becomes a space first) and cuts to `max`
/// characters.
pub fn clean_text(s: &str, max: usize) -> String {
    let t: String = s.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control()).collect();
    t.trim().chars().take(max).collect::<String>().trim().to_string()
}

fn tag(p: &Probe, key: &str, max: usize) -> Option<String> {
    p.tags.get(key).map(|v| clean_text(v, max)).filter(|v| !v.is_empty())
}

/// First four consecutive digits in `s` as a year in 1..=9999.
pub fn year_from(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let start = (0..b.len().saturating_sub(3)).find(|&i| b[i..i + 4].iter().all(u8::is_ascii_digit))?;
    let y: i64 = s[start..start + 4].parse().ok()?;
    (1..=9999).contains(&y).then_some(y)
}

fn file_stem(file_name: &str) -> String {
    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
    clean_text(stem, MAX_NAME)
}

/// Maps file tags to the editable track fields (plan section 2.6). `file_name` is the bare
/// name of the source, the title fallback.
pub fn edits_from_tags(p: &Probe, file_name: &str) -> TrackEdits {
    let title = tag(p, "title", MAX_NAME)
        .or_else(|| Some(file_stem(file_name)).filter(|s| !s.is_empty()))
        .or_else(|| Some(clean_text(file_name, MAX_NAME)).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "Untitled".to_string());
    let composers = p
        .tags
        .get("composer")
        .map(|c| {
            c.split([';', '/'])
                .map(|x| clean_text(x, MAX_NAME))
                .filter(|x| !x.is_empty())
                .take(MAX_COMPOSERS)
                .collect()
        })
        .unwrap_or_default();
    TrackEdits {
        band: tag(p, "artist", MAX_NAME).or_else(|| tag(p, "album_artist", MAX_NAME)).unwrap_or_default(),
        album: tag(p, "album", MAX_NAME).unwrap_or_default(),
        title,
        composers,
        year: p.tags.get("date").and_then(|d| year_from(d)).or_else(|| p.tags.get("year").and_then(|d| year_from(d))),
        source_url: None,
        copyright: tag(p, "copyright", MAX_COPYRIGHT),
    }
}

// ---- conversion ----

/// Name of the finished file inside the job folder.
pub const AUDIO_FLAC: &str = "audio.flac";
const PART_NAME: &str = ".audio.flac.part";

/// Converts the first audio stream of `src` to FLAC, 44.1 kHz stereo, as `<job_dir>/audio.flac`
/// (written to `.audio.flac.part` first, then renamed; the part file is removed on error or
/// cancel). Refuses sources longer than 15 minutes before starting. `src` is only read.
pub fn to_flac(ffmpeg: &Path, src: &Path, probe: &Probe, job_dir: &Path, cancel: &Cancel) -> Result<PathBuf, MediaError> {
    require_absolute(src)?;
    check_duration(probe, &bare_name(src))?;
    let part = job_dir.join(PART_NAME);
    let dest = job_dir.join(AUDIO_FLAC);
    // a part file here is a leftover of ours (the folder is the job's own)
    if part.symlink_metadata().is_ok() {
        std::fs::remove_file(&part).map_err(|e| MediaError::Failed(format!("cannot clear {PART_NAME}: {e}")))?;
    }
    let mut args: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"].iter().map(|s| s.to_string()).collect();
    args.push(file_arg(src));
    args.extend(["-map", "0:a:0", "-vn", "-sn", "-dn", "-c:a", "flac", "-ar", "44100", "-ac", "2", "-f", "flac"].iter().map(|s| s.to_string()));
    args.push(part.to_string_lossy().into_owned());
    let result = (|| {
        let out = run(ffmpeg, &args, cancel)?;
        if !out.success {
            let msg: String = out.stderr_last.chars().filter(|c| !c.is_control()).take(MAX_ERROR_CHARS).collect();
            return failed(if msg.is_empty() { "Conversion failed".to_string() } else { format!("Conversion failed: {msg}") });
        }
        let mut magic = [0u8; 4];
        let ok = std::fs::File::open(&part)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic))
            .is_ok()
            && &magic == b"fLaC";
        if !ok {
            return failed("Conversion failed: ffmpeg wrote no FLAC audio");
        }
        std::fs::rename(&part, &dest).map_err(|e| MediaError::Failed(format!("cannot finish {AUDIO_FLAC}: {e}")))?;
        Ok(dest.clone())
    })();
    if result.is_err() && part.symlink_metadata().is_ok() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import").join(name)
    }

    fn tool(name: &str) -> PathBuf {
        crate::tools::find_in_path(name, &std::env::var_os("PATH").unwrap_or_default())
            .unwrap_or_else(|| panic!("{name} is needed for these tests"))
    }

    fn probe_fixture(name: &str) -> Result<Probe, MediaError> {
        probe(&tool("ffprobe"), &fixture(name), &Cancel::new())
    }

    fn msg(r: Result<impl fmt::Debug, MediaError>) -> String {
        r.unwrap_err().to_string()
    }

    fn tags(pairs: &[(&str, &str)]) -> Probe {
        Probe {
            duration_s: Some(10.0),
            has_audio: true,
            has_video: false,
            tags: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn mp3_tags_map_to_fields() {
        let p = probe_fixture("tagged.mp3").unwrap();
        assert!(p.has_audio && !p.has_video);
        assert!((p.duration_s.unwrap() - 5.0).abs() < 0.2);
        let e = edits_from_tags(&p, "tagged.mp3");
        assert_eq!(e.band, "The Example Band");
        assert_eq!(e.album, "Test Pressings");
        assert_eq!(e.title, "Glass Harbour");
        assert_eq!(e.composers, ["Ann Example", "Bo Sample"]);
        assert_eq!(e.year, Some(2021));
        assert_eq!(e.copyright.as_deref(), Some("(c) 2021 The Example Band"));
        assert_eq!(e.source_url, None);
    }

    #[test]
    fn ogg_stream_tags_map_to_fields() {
        let e = edits_from_tags(&probe_fixture("tagged.ogg").unwrap(), "tagged.ogg");
        assert_eq!((e.band.as_str(), e.title.as_str(), e.year), ("The Example Band", "Glass Harbour", Some(2021)));
        assert_eq!(e.composers, ["Ann Example", "Bo Sample"]);
    }

    #[test]
    fn untagged_file_uses_its_name_as_title() {
        let p = probe_fixture("untagged.flac").unwrap();
        let e = edits_from_tags(&p, "untagged.flac");
        assert_eq!(e.title, "untagged");
        assert_eq!((e.band.as_str(), e.album.as_str(), e.year), ("", "", None));
        assert!(e.composers.is_empty() && e.copyright.is_none());
    }

    #[test]
    fn video_with_audio_is_probed() {
        let p = probe_fixture("with-audio.mp4").unwrap();
        assert!(p.has_audio && p.has_video);
        assert_eq!(edits_from_tags(&p, "with-audio.mp4").title, "Clip Title");
    }

    #[test]
    fn probe_errors_have_the_specified_texts() {
        assert_eq!(msg(probe_fixture("no-audio.mp4")), "Selected file no-audio.mp4 has no audio track");
        assert_eq!(
            msg(probe_fixture("not-audio.mp3")),
            "Selected file not-audio.mp3 could not be read as audio or video"
        );
        assert_eq!(
            msg(probe_fixture("does-not-exist.mp3")),
            "Selected file does-not-exist.mp3 could not be read as audio or video"
        );
        // a folder is not a file
        let dir = tempfile::tempdir().unwrap();
        assert!(msg(probe(&tool("ffprobe"), dir.path(), &Cancel::new())).contains("could not be read"));
        assert!(msg(probe(&tool("ffprobe"), Path::new("relative.mp3"), &Cancel::new())).contains("absolute"));
    }

    #[test]
    fn long_source_is_refused_before_conversion() {
        let p = probe_fixture("long.flac").unwrap();
        assert!(p.duration_s.unwrap() > MAX_DURATION_S);
        let job = tempfile::tempdir().unwrap();
        // a "ffmpeg" that would leave a trace if it were run
        let bin = tempfile::tempdir().unwrap();
        let fake = write_script(bin.path(), "ffmpeg", "touch ran-anyway");
        let e = to_flac(&fake, &fixture("long.flac"), &p, job.path(), &Cancel::new()).unwrap_err();
        assert_eq!(e.to_string(), "Tracks longer than 15 minutes are not supported");
        assert_eq!(fs::read_dir(job.path()).unwrap().count(), 0);
        // exactly at the limit is fine, unknown length is not
        let mut q = tags(&[]);
        q.duration_s = Some(MAX_DURATION_S);
        assert!(check_duration(&q, "a.mp3").is_ok());
        q.duration_s = None;
        assert!(check_duration(&q, "a.mp3").is_err());
    }

    fn sha(p: &Path) -> Vec<u8> {
        fs::read(p).unwrap()
    }

    #[test]
    fn converts_video_audio_to_flac_44k_stereo_and_leaves_the_source_alone() {
        let src = fixture("with-audio.mp4");
        let before = sha(&src);
        let p = probe(&tool("ffprobe"), &src, &Cancel::new()).unwrap();
        let job = tempfile::tempdir().unwrap();
        let out = to_flac(&tool("ffmpeg"), &src, &p, job.path(), &Cancel::new()).unwrap();
        assert_eq!(out, job.path().join("audio.flac"));
        assert_eq!(&fs::read(&out).unwrap()[..4], b"fLaC");
        let names: Vec<_> = fs::read_dir(job.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1, "no part file left: {names:?}");
        let json: serde_json::Value = serde_json::from_str(
            &run(
                &tool("ffprobe"),
                &["-v".into(), "error".into(), "-print_format".into(), "json".into(), "-show_streams".into(), file_arg(&out)],
                &Cancel::new(),
            )
            .unwrap()
            .stdout,
        )
        .unwrap();
        assert_eq!(json["streams"][0]["codec_name"], "flac");
        assert_eq!(json["streams"][0]["sample_rate"], "44100");
        assert_eq!(json["streams"][0]["channels"], 2);
        assert_eq!(before, sha(&src), "the source is read only");
    }

    #[test]
    fn mono_8k_source_becomes_stereo_44k() {
        let src = fixture("untagged.flac");
        let p = probe(&tool("ffprobe"), &src, &Cancel::new()).unwrap();
        let job = tempfile::tempdir().unwrap();
        let out = to_flac(&tool("ffmpeg"), &src, &p, job.path(), &Cancel::new()).unwrap();
        let o = run(
            &tool("ffprobe"),
            &["-v".into(), "error".into(), "-show_entries".into(), "stream=sample_rate,channels".into(), "-of".into(), "csv=p=0".into(), file_arg(&out)],
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(o.stdout.trim(), "44100,2");
    }

    #[test]
    fn a_leftover_part_file_is_cleared_before_converting() {
        let src = fixture("tagged.mp3");
        let p = probe(&tool("ffprobe"), &src, &Cancel::new()).unwrap();
        let job = tempfile::tempdir().unwrap();
        fs::write(job.path().join(".audio.flac.part"), "stale").unwrap();
        let out = to_flac(&tool("ffmpeg"), &src, &p, job.path(), &Cancel::new()).unwrap();
        assert_eq!(&fs::read(out).unwrap()[..4], b"fLaC");
        assert!(!job.path().join(".audio.flac.part").exists());
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn failed_conversion_reports_ffmpegs_message_and_removes_the_part() {
        let bin = tempfile::tempdir().unwrap();
        let job = tempfile::tempdir().unwrap();
        let fake = write_script(
            bin.path(),
            "ffmpeg",
            "for a; do last=$a; done; echo partial > \"$last\"; echo 'first line' >&2; echo 'Error: disk full' >&2; exit 1",
        );
        let e = to_flac(&fake, &fixture("tagged.mp3"), &tags(&[]), job.path(), &Cancel::new()).unwrap_err();
        assert_eq!(e.to_string(), "Conversion failed: Error: disk full");
        assert_eq!(fs::read_dir(job.path()).unwrap().count(), 0);
        // success exit but not FLAC
        let fake = write_script(bin.path(), "ffmpeg2", "for a; do last=$a; done; echo notflac > \"$last\"");
        let e = to_flac(&fake, &fixture("tagged.mp3"), &tags(&[]), job.path(), &Cancel::new()).unwrap_err();
        assert!(e.to_string().starts_with("Conversion failed"), "{e}");
        assert_eq!(fs::read_dir(job.path()).unwrap().count(), 0);
    }

    #[test]
    fn conversion_receives_the_specified_argv() {
        let bin = tempfile::tempdir().unwrap();
        let job = tempfile::tempdir().unwrap();
        let log = bin.path().join("argv");
        let fake = write_script(
            bin.path(),
            "ffmpeg",
            &format!("for a; do printf '%s\\n' \"$a\" >> '{}'; last=$a; done; printf fLaC > \"$last\"", log.display()),
        );
        let src = fixture("tagged.mp3");
        to_flac(&fake, &src, &tags(&[]), job.path(), &Cancel::new()).unwrap();
        let argv: Vec<String> = fs::read_to_string(log).unwrap().lines().map(String::from).collect();
        let part = job.path().join(".audio.flac.part").to_string_lossy().into_owned();
        let want: Vec<String> = format!(
            "-nostdin -hide_banner -loglevel error -n -i file:{} -map 0:a:0 -vn -sn -dn -c:a flac -ar 44100 -ac 2 -f flac {part}",
            src.display()
        )
        .split(' ')
        .map(String::from)
        .collect();
        assert_eq!(argv, want);
    }

    #[test]
    fn cancel_stops_the_conversion_and_removes_the_part() {
        let bin = tempfile::tempdir().unwrap();
        let job = tempfile::tempdir().unwrap();
        let fake = write_script(bin.path(), "ffmpeg", "for a; do last=$a; done; echo x > \"$last\"; sleep 60");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            c2.cancel();
        });
        let started = std::time::Instant::now();
        let e = to_flac(&fake, &fixture("tagged.mp3"), &tags(&[]), job.path(), &cancel).unwrap_err();
        t.join().unwrap();
        assert_eq!(e, MediaError::Cancelled);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(fs::read_dir(job.path()).unwrap().count(), 0);
        // already cancelled: the next tool is stopped at once
        let e = to_flac(&fake, &fixture("tagged.mp3"), &tags(&[]), job.path(), &cancel).unwrap_err();
        assert_eq!(e, MediaError::Cancelled);
    }

    #[test]
    fn oversized_source_is_refused() {
        // sparse file just over 1 GiB (no disk use), probed without running ffprobe
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("huge.flac");
        fs::File::create(&f).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        let bin = tempfile::tempdir().unwrap();
        let fake = write_script(bin.path(), "ffprobe", "touch ran-anyway");
        assert_eq!(msg(probe(&fake, &f, &Cancel::new())), "Selected file huge.flac is larger than 1 GiB");
        assert!(!bin.path().join("ran-anyway").exists());
    }

    // ---- pure mapping ----

    #[test]
    fn mapping_prefers_artist_then_album_artist_and_cleans_values() {
        let e = edits_from_tags(&tags(&[("album_artist", "  Various\tArtists "), ("title", "T\u{7}itle\n2")]), "x.mp3");
        assert_eq!(e.band, "Various Artists");
        assert_eq!(e.title, "Title 2");
        let e = edits_from_tags(&tags(&[("artist", "Solo"), ("album_artist", "Group")]), "x.mp3");
        assert_eq!(e.band, "Solo");
    }

    #[test]
    fn mapping_splits_composers_on_semicolon_and_slash() {
        let e = edits_from_tags(&tags(&[("composer", " A B ; C/D ;; /  ")]), "x.mp3");
        assert_eq!(e.composers, ["A B", "C", "D"]);
        let many = (0..30).map(|i| format!("c{i}")).collect::<Vec<_>>().join(";");
        assert_eq!(edits_from_tags(&tags(&[("composer", &many)]), "x.mp3").composers.len(), 20);
    }

    #[test]
    fn mapping_year_rules() {
        for (v, want) in [
            ("2021-03-04", Some(2021)),
            ("1999", Some(1999)),
            ("March 2005", Some(2005)),
            ("0000", None),
            ("21", None),
            ("abcd", None),
            ("", None),
            ("0001", Some(1)),
        ] {
            let e = edits_from_tags(&tags(&[("date", v)]), "x.mp3");
            assert_eq!(e.year, want, "{v:?}");
        }
        assert_eq!(edits_from_tags(&tags(&[("year", "1987")]), "x.mp3").year, Some(1987));
        assert_eq!(edits_from_tags(&tags(&[("date", "garbage"), ("year", "1987")]), "x.mp3").year, Some(1987));
    }

    #[test]
    fn mapping_title_fallbacks_and_limits() {
        assert_eq!(edits_from_tags(&tags(&[("title", "   ")]), "My Song.final.mp3").title, "My Song.final");
        assert_eq!(edits_from_tags(&tags(&[]), ".mp3").title, ".mp3");
        assert_eq!(edits_from_tags(&tags(&[]), "").title, "Untitled");
        let long = "x".repeat(500);
        let e = edits_from_tags(&tags(&[("title", &long), ("album", &long), ("artist", &long), ("copyright", &long)]), "a.mp3");
        assert_eq!((e.title.len(), e.album.len(), e.band.len()), (200, 200, 200));
        assert_eq!(e.copyright.unwrap().len(), 500);
        let multibyte = "é".repeat(300);
        assert_eq!(edits_from_tags(&tags(&[("title", &multibyte)]), "a.mp3").title.chars().count(), 200);
        // every mapped value passes the strict write rules
        let weird = edits_from_tags(&tags(&[("title", "\u{0}\u{1}  x "), ("composer", "\u{2}")]), "a.mp3");
        assert_eq!(weird.title, "x");
        assert!(weird.composers.is_empty());
    }

    #[test]
    fn tag_names_are_case_insensitive_and_container_tags_win() {
        let json = r#"{"streams":[{"codec_type":"audio","duration":"5.0","tags":{"TITLE":"Stream","Artist":"S Artist"}}],
                       "format":{"duration":"5.0","tags":{"title":"Container"}}}"#;
        let p = parse_probe(json).unwrap();
        let e = edits_from_tags(&p, "f.ogg");
        assert_eq!((e.title.as_str(), e.band.as_str()), ("Container", "S Artist"));
    }

    #[test]
    fn parse_probe_handles_odd_json() {
        assert!(parse_probe("").is_none());
        assert!(parse_probe("[]").is_none());
        assert!(parse_probe("{}").is_none());
        let p = parse_probe(r#"{"streams":[{"codec_type":"video","disposition":{"attached_pic":1}}],"format":{}}"#).unwrap();
        assert!(!p.has_audio && !p.has_video && p.duration_s.is_none());
        let p = parse_probe(r#"{"streams":[{"codec_type":"audio","duration":"-3"}],"format":{"duration":"nan"}}"#).unwrap();
        assert_eq!(p.duration_s, None);
        let p = parse_probe(r#"{"streams":[{"codec_type":"audio","duration":"7.5"}],"format":{}}"#).unwrap();
        assert_eq!(p.duration_s, Some(7.5));
    }
}
