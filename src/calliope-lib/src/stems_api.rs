//! "calliope-stems API v1": the JSON types, limits and validation shared by the client and
//! the server, and a minimal FLAC STREAMINFO reader used to check uploads.

use std::io::Read;

use serde::{Deserialize, Serialize};

/// Value of `Health::service`.
pub const SERVICE_NAME: &str = "calliope-stems";
pub const API_VERSION: u32 = 1;
/// Longest audio the system accepts, in seconds (15 minutes).
pub const MAX_DURATION_S: u64 = 900;
/// Most stems a job may produce.
pub const MAX_STEMS: usize = 16;

/// `GET /v1/health`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub service: String,
    pub api: u32,
    pub version: String,
    pub models: Vec<String>,
    pub default_model: String,
    pub busy: bool,
    pub max_duration_s: u64,
    pub max_upload_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    /// No further change will happen.
    pub fn is_final(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed | JobState::Cancelled)
    }
}

/// `202` body of `POST /v1/jobs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobCreated {
    pub job: String,
    pub state: JobState,
}

/// `GET /v1/jobs/<id>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStatus {
    pub job: String,
    pub state: JobState,
    pub progress: Option<f64>,
    pub stems: Option<Vec<String>>,
    pub error: Option<String>,
    /// Peak of every stem the server measured; only on a `done` job, and the key is omitted when
    /// the server measured nothing (an old server, or `--no-stem-peaks`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stem_peaks: Option<Vec<StemPeak>>,
}

/// One measured stem. `peak` (max |sample| over all channels) and `bits` are the data: the app
/// decides with `flac_peak::is_below(peak, bits, ..)`. `peak_dbfs` is for humans (`null` for
/// digital silence) and is ignored by the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StemPeak {
    pub name: String,
    pub peak: u64,
    pub bits: u32,
    pub peak_dbfs: Option<f64>,
}

/// Checks a server's peak list against its stem list: every name is in `stems`, no duplicates,
/// `bits` in 4..=32 and `peak <= 2^(bits-1)`. A stem without an entry is "not measured".
pub fn check_stem_peaks(stems: &[String], peaks: &[StemPeak]) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for p in peaks {
        if !stems.contains(&p.name) {
            return Err(format!("peak for unknown stem {:?}", p.name));
        }
        if seen.contains(&p.name.as_str()) {
            return Err(format!("duplicate peak for {:?}", p.name));
        }
        seen.push(&p.name);
        if !(4..=32).contains(&p.bits) {
            return Err(format!("bits {} out of range", p.bits));
        }
        if p.peak > 1u64 << (p.bits - 1) {
            return Err(format!("peak {} too large for {} bits", p.peak, p.bits));
        }
    }
    Ok(())
}

/// Body of every 4xx/5xx answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

/// A server-generated job id: 1 to 64 characters of `[0-9a-f-]`, starting with a hex digit
/// (a hyphenated UUID qualifies). Checked before an id is used in a URL or a file name.
pub fn is_valid_job_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0] != b'-'
        && b.iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c) || *c == b'-')
}

/// A stem name: `^[a-z0-9][a-z0-9_-]{0,31}$`.
pub fn is_valid_stem_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() || b.len() > 32 {
        return false;
    }
    let ok = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    ok(b[0]) && b[1..].iter().all(|&c| ok(c) || c == b'_' || c == b'-')
}

/// What the STREAMINFO block says about a FLAC stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlacInfo {
    pub sample_rate: u32,
    pub channels: u8,
    pub total_samples: u64,
    pub duration_s: f64,
}

#[derive(Debug)]
pub enum FlacError {
    /// The data does not start with `fLaC`.
    NotFlac,
    /// The first metadata block is not a 34-byte STREAMINFO.
    NoStreamInfo,
    /// The data ends inside the header.
    Truncated,
    /// STREAMINFO has total samples 0 (unknown length).
    UnknownLength,
    /// STREAMINFO has sample rate 0.
    BadSampleRate,
    Io(std::io::Error),
}

impl std::fmt::Display for FlacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlacError::NotFlac => write!(f, "not a FLAC file"),
            FlacError::NoStreamInfo => write!(f, "FLAC file has no valid STREAMINFO block"),
            FlacError::Truncated => write!(f, "FLAC header is truncated"),
            FlacError::UnknownLength => write!(f, "FLAC file does not state its length"),
            FlacError::BadSampleRate => write!(f, "FLAC file has an invalid sample rate"),
            FlacError::Io(e) => write!(f, "cannot read FLAC header: {e}"),
        }
    }
}

impl std::error::Error for FlacError {}

fn read_exact_or_truncated(r: &mut impl Read, buf: &mut [u8]) -> Result<(), FlacError> {
    r.read_exact(buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            FlacError::Truncated
        } else {
            FlacError::Io(e)
        }
    })
}

/// Reads the `fLaC` magic and the STREAMINFO block (which must come first) from the start of
/// `reader`. Consumes at most 42 bytes.
pub fn flac_info(reader: &mut impl Read) -> Result<FlacInfo, FlacError> {
    let mut magic = [0u8; 4];
    match reader.read_exact(&mut magic) {
        Ok(()) => {}
        // Fewer than four bytes cannot be FLAC.
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(FlacError::NotFlac),
        Err(e) => return Err(FlacError::Io(e)),
    }
    if &magic != b"fLaC" {
        return Err(FlacError::NotFlac);
    }
    let mut head = [0u8; 4];
    read_exact_or_truncated(reader, &mut head)?;
    let block_type = head[0] & 0x7f;
    let len = u32::from_be_bytes([0, head[1], head[2], head[3]]);
    if block_type != 0 || len != 34 {
        return Err(FlacError::NoStreamInfo);
    }
    let mut b = [0u8; 34];
    read_exact_or_truncated(reader, &mut b)?;
    // Bytes 0..10: block and frame sizes. Then 20 bits sample rate, 3 bits channels - 1,
    // 5 bits bits-per-sample - 1, 36 bits total samples.
    let sample_rate = (u32::from(b[10]) << 12) | (u32::from(b[11]) << 4) | u32::from(b[12] >> 4);
    let channels = ((b[12] >> 1) & 0x07) + 1;
    let total_samples =
        (u64::from(b[13] & 0x0f) << 32) | u64::from(u32::from_be_bytes([b[14], b[15], b[16], b[17]]));
    if sample_rate == 0 {
        return Err(FlacError::BadSampleRate);
    }
    if total_samples == 0 {
        return Err(FlacError::UnknownLength);
    }
    Ok(FlacInfo {
        sample_rate,
        channels,
        total_samples,
        duration_s: total_samples as f64 / f64::from(sample_rate),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../calliope-gui/tests/fixtures/import").join(name)
    }

    fn info_of(name: &str) -> Result<FlacInfo, FlacError> {
        flac_info(&mut std::fs::File::open(fixture(name)).unwrap())
    }

    /// A header with the given STREAMINFO values.
    fn header(rate: u32, channels: u8, total: u64) -> Vec<u8> {
        let mut v = b"fLaC".to_vec();
        v.extend([0x80, 0, 0, 34]);
        let mut si = [0u8; 34];
        si[10] = (rate >> 12) as u8;
        si[11] = (rate >> 4) as u8;
        si[12] = (((rate & 0x0f) as u8) << 4) | (((channels - 1) & 7) << 1);
        si[13] = ((total >> 32) & 0x0f) as u8;
        si[14..18].copy_from_slice(&(total as u32).to_be_bytes());
        v.extend(si);
        v
    }

    #[test]
    fn fixtures_streaminfo() {
        let u = info_of("untagged.flac").unwrap();
        assert_eq!((u.sample_rate, u.channels), (8000, 1));
        assert!((u.duration_s - 5.0).abs() < 0.01, "{u:?}");
        let l = info_of("long.flac").unwrap();
        assert!(l.duration_s > 900.0 && l.duration_s < 1000.0, "{l:?}");
        let s = info_of("stems/vocals.flac").unwrap();
        assert!((s.duration_s - 1.0).abs() < 0.01);
        assert!(matches!(info_of("not-flac.flac"), Err(FlacError::NotFlac)));
        assert!(matches!(info_of("not-audio.mp3"), Err(FlacError::NotFlac)));
    }

    #[test]
    fn synthetic_headers() {
        let i = flac_info(&mut header(44100, 2, 44100 * 90).as_slice()).unwrap();
        assert_eq!((i.sample_rate, i.channels, i.total_samples), (44100, 2, 3_969_000));
        assert!((i.duration_s - 90.0).abs() < 1e-9);
        // total samples above 32 bits
        let big = (1u64 << 33) + 5;
        assert_eq!(flac_info(&mut header(96000, 8, big).as_slice()).unwrap().total_samples, big);
        assert!(matches!(flac_info(&mut header(44100, 2, 0).as_slice()), Err(FlacError::UnknownLength)));
        assert!(matches!(flac_info(&mut header(0, 2, 10).as_slice()), Err(FlacError::BadSampleRate)));
    }

    #[test]
    fn malformed_headers() {
        assert!(matches!(flac_info(&mut &b""[..]), Err(FlacError::NotFlac)));
        assert!(matches!(flac_info(&mut &b"fLa"[..]), Err(FlacError::NotFlac)));
        assert!(matches!(flac_info(&mut &b"OggS\0\0\0\0"[..]), Err(FlacError::NotFlac)));
        assert!(matches!(flac_info(&mut &b"fLaC"[..]), Err(FlacError::Truncated)));
        let full = header(44100, 2, 100);
        assert!(matches!(flac_info(&mut &full[..20]), Err(FlacError::Truncated)));
        let mut not_first = full.clone();
        not_first[4] = 0x84; // VORBIS_COMMENT first
        assert!(matches!(flac_info(&mut not_first.as_slice()), Err(FlacError::NoStreamInfo)));
        let mut wrong_len = full;
        wrong_len[7] = 33;
        assert!(matches!(flac_info(&mut wrong_len.as_slice()), Err(FlacError::NoStreamInfo)));
    }

    #[test]
    fn job_ids() {
        for ok in ["0199c1a2-0000-4000-8000-000000000001", "abc", "0", &"a".repeat(64)] {
            assert!(is_valid_job_id(ok), "{ok}");
        }
        for bad in ["", "-abc", "ABC", "a/b", "a b", "..", "a.b", "g", "a\n", "é", &"a".repeat(65)] {
            assert!(!is_valid_job_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn stem_names() {
        for ok in ["vocals", "a", "0", "bass-2", "lead_gtr", &"a".repeat(32)] {
            assert!(is_valid_stem_name(ok), "{ok}");
        }
        for bad in ["", "_a", "-a", "Vocals", "a.b", "a/b", "a b", "..", "é", &"a".repeat(33), "a\n"] {
            assert!(!is_valid_stem_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn serde_round_trips() {
        let h = Health {
            service: SERVICE_NAME.into(),
            api: API_VERSION,
            version: "0.1.0".into(),
            models: vec!["htdemucs_6s".into()],
            default_model: "htdemucs_6s".into(),
            busy: false,
            max_duration_s: MAX_DURATION_S,
            max_upload_bytes: 314_572_800,
        };
        let text = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Health>(&text).unwrap(), h);
        let doc = r#"{"service":"calliope-stems","api":1,"version":"x","models":["m"],"default_model":"m","busy":true,"max_duration_s":900,"max_upload_bytes":1}"#;
        assert!(serde_json::from_str::<Health>(doc).unwrap().busy);

        assert_eq!(serde_json::to_string(&JobCreated { job: "ab".into(), state: JobState::Queued }).unwrap(),
            r#"{"job":"ab","state":"queued"}"#);
        for (s, name) in [
            (JobState::Queued, "queued"),
            (JobState::Running, "running"),
            (JobState::Done, "done"),
            (JobState::Failed, "failed"),
            (JobState::Cancelled, "cancelled"),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{name}\""));
            assert_eq!(serde_json::from_str::<JobState>(&format!("\"{name}\"")).unwrap(), s);
        }
        assert!(serde_json::from_str::<JobState>("\"Queued\"").is_err());
        assert!(JobState::Done.is_final() && !JobState::Running.is_final());

        let st = JobStatus {
            job: "ab".into(),
            state: JobState::Done,
            progress: Some(0.42),
            stems: Some(vec!["vocals".into()]),
            error: None,
            stem_peaks: None,
        };
        let text = serde_json::to_string(&st).unwrap();
        assert!(text.contains("\"error\":null"));
        assert_eq!(serde_json::from_str::<JobStatus>(&text).unwrap(), st);
        let queued: JobStatus =
            serde_json::from_str(r#"{"job":"ab","state":"queued","progress":null,"stems":null,"error":null}"#).unwrap();
        assert_eq!(queued.progress, None);
        let e = ErrorBody { error: "bad".into() };
        assert_eq!(serde_json::from_str::<ErrorBody>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
    }

    fn sp(name: &str, peak: u64, bits: u32) -> StemPeak {
        StemPeak { name: name.into(), peak, bits, peak_dbfs: None }
    }

    #[test]
    fn stem_peaks_serde_and_compat() {
        let base = r#"{"job":"ab","state":"done","progress":1.0,"stems":["vocals","piano"],"error":null"#;
        let absent: JobStatus = serde_json::from_str(&format!("{base}}}")).unwrap();
        assert_eq!(absent.stem_peaks, None);
        let null: JobStatus = serde_json::from_str(&format!(r#"{base},"stem_peaks":null}}"#)).unwrap();
        assert_eq!(null.stem_peaks, None);
        assert!(!serde_json::to_string(&absent).unwrap().contains("stem_peaks"));
        let full = format!(
            r#"{base},"stem_peaks":[{{"name":"vocals","peak":21450,"bits":16,"peak_dbfs":-3.68}},{{"name":"piano","peak":0,"bits":16,"peak_dbfs":null}}]}}"#
        );
        let st: JobStatus = serde_json::from_str(&full).unwrap();
        let peaks = st.stem_peaks.clone().unwrap();
        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[1], StemPeak { name: "piano".into(), peak: 0, bits: 16, peak_dbfs: None });
        let text = serde_json::to_string(&st).unwrap();
        assert!(text.contains("\"stem_peaks\""));
        assert_eq!(serde_json::from_str::<JobStatus>(&text).unwrap(), st);

        // The JobStatus of f62ecf9 (no deny_unknown_fields) still parses the new JSON.
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct OldJobStatus {
            job: String,
            state: JobState,
            progress: Option<f64>,
            stems: Option<Vec<String>>,
            error: Option<String>,
        }
        let old: OldJobStatus = serde_json::from_str(&full).unwrap();
        assert_eq!(old.stems.unwrap().len(), 2);
    }

    #[test]
    fn check_stem_peaks_rules() {
        let stems: Vec<String> = vec!["vocals".into(), "piano".into()];
        assert!(check_stem_peaks(&stems, &[]).is_ok());
        assert!(check_stem_peaks(&stems, &[sp("piano", 0, 16)]).is_ok());
        assert!(check_stem_peaks(&stems, &[sp("vocals", 1 << 15, 16), sp("piano", 1 << 31, 32)]).is_ok());
        assert!(check_stem_peaks(&stems, &[sp("vocals", 5, 4), sp("piano", 8, 4)]).is_ok());
        assert!(check_stem_peaks(&stems, &[sp("drums", 1, 16)]).is_err());
        assert!(check_stem_peaks(&stems, &[sp("piano", 1, 16), sp("piano", 2, 16)]).is_err());
        assert!(check_stem_peaks(&stems, &[sp("piano", 1, 3)]).is_err());
        assert!(check_stem_peaks(&stems, &[sp("piano", 1, 33)]).is_err());
        assert!(check_stem_peaks(&stems, &[sp("piano", (1 << 15) + 1, 16)]).is_err());
        assert!(check_stem_peaks(&stems, &[sp("piano", 9, 4)]).is_err());
    }
}
