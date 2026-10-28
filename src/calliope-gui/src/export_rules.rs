//! Pure rules of the Export feature: which file is exported, memory numbers, RC-5 and file
//! names, ffprobe output, output formats, tags and remembered folders. std + serde only.
//! See plan sections 2.2 to 2.5 and 2.7 of `specs/gui-export.plan.md`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::repository::{cut_bytes, sanitize_component, TrackRecord};
use crate::track_meta::TrackType;

/// Shown (and returned by Rust) when the track has no tempo map: the count-in needs one.
pub const NO_TEMPO_REASON: &str =
    "Set a tempo in the Player first: the export starts with a count-in";

/// Where an export goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportKind {
    Rc5,
    Mp3,
    Ogg,
}

impl ExportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExportKind::Rc5 => "rc5",
            ExportKind::Mp3 => "mp3",
            ExportKind::Ogg => "ogg",
        }
    }
}

// ---- 2.2: which file is exported ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// The first saved backing mix of a stem track.
    Mix,
    /// The audio file of a backing track.
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSource {
    pub kind: SourceKind,
    /// Relative to the track folder.
    pub file: String,
}

/// The backing track of `t`, or the reason why there is none (shown in the pane). Only the
/// first backing variant is used; stems, the original and tablatures are never exported.
pub fn export_source(t: &TrackRecord) -> Result<ExportSource, String> {
    let is_missing = |f: &str| t.missing.iter().any(|m| m == f);
    match t.track_type {
        TrackType::Stem => {
            let first = t
                .backings
                .first()
                .ok_or("No mix saved yet: save one in the Editor")?;
            if is_missing(&first.file) {
                return Err(format!(
                    "The saved mix ({}) is missing: save it again in the Editor",
                    first.file
                ));
            }
            Ok(ExportSource {
                kind: SourceKind::Mix,
                file: first.file.clone(),
            })
        }
        TrackType::Backing => {
            let audio = t.audio.as_ref().ok_or("This track has no audio file")?;
            if is_missing(audio) {
                return Err(format!("The audio file ({audio}) is missing"));
            }
            Ok(ExportSource {
                kind: SourceKind::Audio,
                file: audio.clone(),
            })
        }
    }
}

// ---- 2.5: RC-5 memory and paths ----

const MEMORY_MESSAGE: &str = "Enter a memory number from 1 to 99";

/// The memory number typed by the user: 1 to 3 ASCII digits (after trimming), value 1..=99.
pub fn parse_memory(text: &str) -> Result<u8, String> {
    let t = text.trim();
    if t.is_empty() || t.len() > 3 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MEMORY_MESSAGE.into());
    }
    match t.parse::<u8>() {
        Ok(n) if (1..=99).contains(&n) => Ok(n),
        _ => Err(MEMORY_MESSAGE.into()),
    }
}

/// `007_1`
pub fn rc5_folder_name(n: u8) -> String {
    format!("{n:03}_1")
}

/// `<base>/ROLAND/WAVE/007_1/007_1.WAV`
pub fn rc5_target(base: &Path, n: u8) -> PathBuf {
    let f = rc5_folder_name(n);
    base.join("ROLAND")
        .join("WAVE")
        .join(&f)
        .join(format!("{f}.WAV"))
}

/// The suggested memory: 1 at first, then the last one plus one (at most 99).
pub fn suggest_memory(last: Option<u8>) -> u8 {
    match last {
        None => 1,
        Some(n) => n.saturating_add(1).min(99),
    }
}

/// The Memory box default: the track's recorded memory when it has one, else the suggestion
/// after the last one used.
pub fn memory_for(recorded: Option<u8>, last: Option<u8>) -> u8 {
    recorded.unwrap_or_else(|| suggest_memory(last))
}

// ---- file names (MP3 / OGG) ----

/// `Band - Title.ext`, or just `Title.ext` without a band. Each part is made safe like the
/// Library's folder export (`/ \ < > : " | ? *` and control characters become `_`); the stem
/// is cut to 240 bytes on a UTF-8 boundary; `fallback` (the track id) is used when nothing is left.
pub fn file_name(band: &str, title: &str, ext: &str, fallback: &str) -> String {
    let parts: Vec<String> = [band, title]
        .iter()
        .map(|p| sanitize_component(p))
        .filter(|p| !p.is_empty())
        .collect();
    let joined = parts.join(" - ");
    let stem = cut_bytes(&joined, 240).trim();
    let stem = if stem.is_empty() { fallback } else { stem };
    format!("{stem}.{ext}")
}

// ---- 2.4: ffprobe, output format, tags, encoders ----

#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub rate: u32,
    pub channels: u32,
    pub duration_s: f64,
}

/// Reads `ffprobe -of json` output (`-show_entries stream=sample_rate,channels,duration:format=duration`).
/// A stream duration wins over the format's. `None` when the rate, the channels or a positive
/// duration cannot be found.
pub fn parse_stream_info(json: &str) -> Option<StreamInfo> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let stream = v.get("streams")?.as_array()?.first()?;
    let number = |x: Option<&serde_json::Value>| -> Option<f64> {
        match x? {
            serde_json::Value::String(s) => s.trim().parse().ok(),
            serde_json::Value::Number(n) => n.as_f64(),
            _ => None,
        }
    };
    let positive = |x: Option<f64>| x.filter(|d| d.is_finite() && *d > 0.0);
    let rate = positive(number(stream.get("sample_rate")))? as u32;
    let channels = positive(number(stream.get("channels")))? as u32;
    let duration_s = positive(number(stream.get("duration")))
        .or_else(|| positive(number(v.get("format").and_then(|f| f.get("duration")))))?;
    (rate > 0 && channels > 0).then_some(StreamInfo {
        rate,
        channels,
        duration_s,
    })
}

/// Sample rates libmp3lame accepts.
const MP3_RATES: [u32; 9] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];

/// The `(rate, channels)` the export is rendered in. RC-5: always 44.1 kHz stereo. MP3: the
/// source rate when libmp3lame supports it, else 44100 for multiples of 11025 and 48000 otherwise.
/// OGG: the source rate. MP3/OGG keep a mono source mono; anything else becomes stereo.
pub fn output_format(kind: ExportKind, src: &StreamInfo) -> (u32, u32) {
    let channels = if src.channels == 1 { 1 } else { 2 };
    match kind {
        ExportKind::Rc5 => (44100, 2),
        ExportKind::Ogg => (src.rate, channels),
        ExportKind::Mp3 => {
            let rate = if MP3_RATES.contains(&src.rate) {
                src.rate
            } else if src.rate.is_multiple_of(11025) {
                44100
            } else {
                48000
            };
            (rate, channels)
        }
    }
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/// The metadata names (ffmpeg `-metadata` keys) and values written to MP3/OGG files: `title`
/// always, `artist` (band) and `album` when not empty, `track` only for a track with an album,
/// `date` when the year is known.
pub fn tags(t: &TrackRecord) -> Vec<(&'static str, String)> {
    let mut out = vec![("title", clean(&t.title))];
    let band = clean(&t.band);
    if !band.is_empty() {
        out.push(("artist", band));
    }
    let album = clean(&t.album);
    if !album.is_empty() {
        out.push(("album", album));
        if let Some(p) = t.position {
            out.push(("track", p.to_string()));
        }
    }
    if let Some(y) = t.year {
        out.push(("date", y.to_string()));
    }
    out
}

/// True when the `ffmpeg -encoders` listing has an encoder called `name` (the second
/// whitespace-separated field of a line).
pub fn has_encoder(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(name))
}

// ---- 2.7: dialog start folder ----

/// The folder a dialog starts in: the remembered folder when it is a folder (`"last"`), else
/// home (`"home"`), else none. A missing remembered folder is not forgotten (an unplugged RC-5
/// is normal).
pub fn start_dir(
    remembered: Option<&Path>,
    home: Option<&Path>,
) -> (&'static str, Option<PathBuf>) {
    if let Some(r) = remembered.filter(|r| r.is_dir()) {
        return ("last", Some(r.to_path_buf()));
    }
    match home {
        Some(h) => ("home", Some(h.to_path_buf())),
        None => ("none", None),
    }
}

/// The start folder of an MP3/OGG dialog: the folder of the file recorded for that kind when
/// it still exists (`"recorded"`), else what `start_dir` gives.
pub fn file_start_dir(
    recorded: Option<&Path>,
    remembered: Option<&Path>,
    home: Option<&Path>,
) -> (&'static str, Option<PathBuf>) {
    if let Some(parent) = recorded.and_then(Path::parent).filter(|p| p.is_dir()) {
        return ("recorded", Some(parent.to_path_buf()));
    }
    start_dir(remembered, home)
}

/// The folder an RC-5 export may use without the dialog, or why not:
/// "none" (no reuse asked or nothing remembered), "other" (reuse != remembered),
/// "gone" (not a directory any more), "no-roland" (no ROLAND directory in it).
pub fn quick_rc5_dir(
    reuse: Option<&str>,
    remembered: Option<&Path>,
) -> Result<PathBuf, &'static str> {
    let (Some(reuse), Some(remembered)) = (reuse, remembered) else {
        return Err("none");
    };
    if remembered.to_string_lossy() != reuse {
        return Err("other");
    }
    if !remembered.is_dir() {
        return Err("gone");
    }
    if !remembered.join("ROLAND").is_dir() {
        return Err("no-roland");
    }
    Ok(remembered.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track_meta::BackingVariant;

    fn variant(file: &str) -> BackingVariant {
        BackingVariant {
            id: "backing".into(),
            name: "Backing".into(),
            file: file.into(),
            created: String::new(),
            modified: String::new(),
            sample_rate: 44100,
            bits: 16,
            mix: serde_json::Value::Null,
            loudness: None,
        }
    }

    fn rec(track_type: TrackType) -> TrackRecord {
        TrackRecord {
            id: "0199c0a0-0000-7000-8000-000000000401".into(),
            track_type,
            band: "The Example Band".into(),
            album: "Export Tests".into(),
            album_position: None,
            title: "Stem Mix".into(),
            position: None,
            composers: vec![],
            year: None,
            source_url: None,
            copyright: None,
            audio: None,
            original: None,
            stems: vec![],
            stem_model: None,
            tablatures: vec![],
            backings: vec![],
            imported: String::new(),
            modified: String::new(),
            revision: String::new(),
            exports: Default::default(),
            missing: vec![],
        }
    }

    #[test]
    fn export_source_table() {
        let mut t = rec(TrackType::Stem);
        assert_eq!(
            export_source(&t).unwrap_err(),
            "No mix saved yet: save one in the Editor"
        );
        t.backings = vec![
            variant("backings/backing.flac"),
            variant("backings/other.flac"),
        ];
        assert_eq!(
            export_source(&t).unwrap(),
            ExportSource {
                kind: SourceKind::Mix,
                file: "backings/backing.flac".into()
            }
        );
        t.missing = vec!["backings/other.flac".into()];
        assert_eq!(
            export_source(&t).unwrap().file,
            "backings/backing.flac",
            "only the first variant counts"
        );
        t.missing = vec!["backings/backing.flac".into()];
        assert_eq!(
            export_source(&t).unwrap_err(),
            "The saved mix (backings/backing.flac) is missing: save it again in the Editor"
        );
        // A stem track's own audio / stems are never used.
        t.audio = Some("x.flac".into());
        t.backings.clear();
        assert!(export_source(&t).is_err());

        let mut b = rec(TrackType::Backing);
        assert_eq!(
            export_source(&b).unwrap_err(),
            "This track has no audio file"
        );
        b.audio = Some("backing.mp3".into());
        assert_eq!(
            export_source(&b).unwrap(),
            ExportSource {
                kind: SourceKind::Audio,
                file: "backing.mp3".into()
            }
        );
        b.audio = Some("gone.flac".into());
        b.missing = vec!["gone.flac".into()];
        assert_eq!(
            export_source(&b).unwrap_err(),
            "The audio file (gone.flac) is missing"
        );
    }

    #[test]
    fn memory_numbers() {
        for (text, n) in [("1", 1), (" 7 ", 7), ("007", 7), ("99", 99), ("42", 42)] {
            assert_eq!(parse_memory(text), Ok(n), "{text:?}");
        }
        for text in ["0", "100", "abc", "", "7.5", "-3", "1000", "00", "+5", "٣"] {
            assert_eq!(
                parse_memory(text).unwrap_err(),
                "Enter a memory number from 1 to 99",
                "{text:?}"
            );
        }
        assert_eq!(suggest_memory(None), 1);
        assert_eq!(suggest_memory(Some(7)), 8);
        assert_eq!(suggest_memory(Some(98)), 99);
        assert_eq!(suggest_memory(Some(99)), 99);
        assert_eq!(memory_for(Some(7), Some(98)), 7);
        assert_eq!(memory_for(Some(7), None), 7);
        assert_eq!(memory_for(None, Some(98)), 99);
        assert_eq!(memory_for(None, None), 1);
    }

    #[test]
    fn rc5_paths() {
        assert_eq!(rc5_folder_name(7), "007_1");
        assert_eq!(
            rc5_target(Path::new("/b"), 7),
            PathBuf::from("/b/ROLAND/WAVE/007_1/007_1.WAV")
        );
        assert_eq!(
            rc5_target(Path::new("/b"), 99),
            PathBuf::from("/b/ROLAND/WAVE/099_1/099_1.WAV")
        );
    }

    #[test]
    fn file_names() {
        assert_eq!(
            file_name("AC/DC*", "Live: A/B?", "mp3", "id"),
            "AC_DC_ - Live_ A_B_.mp3"
        );
        assert_eq!(file_name("", "T", "ogg", "id"), "T.ogg");
        assert_eq!(
            file_name("The Example Band", "Stem Mix", "mp3", "id"),
            "The Example Band - Stem Mix.mp3"
        );
        assert_eq!(file_name("", "", "mp3", "0199-id"), "0199-id.mp3");
        assert_eq!(file_name("  ", "...", "ogg", "0199-id"), "0199-id.ogg");
        assert_eq!(file_name("Band", "", "ogg", "id"), "Band.ogg");
        let long = file_name("", &"é".repeat(150), "mp3", "id"); // 300 bytes
        let stem = long.strip_suffix(".mp3").unwrap();
        assert!(
            stem.len() <= 240 && !stem.is_empty() && stem.chars().all(|c| c == 'é'),
            "{}",
            stem.len()
        );
        let long = file_name(&"é".repeat(120), &"ü".repeat(120), "mp3", "id");
        assert!(long.strip_suffix(".mp3").unwrap().len() <= 240);
    }

    #[test]
    fn stream_info_from_ffprobe_json() {
        let flac = r#"{"programs": [], "stream_groups": [], "streams": [{"sample_rate": "48000", "channels": 1, "duration": "4.000000"}], "format": {"duration": "4.000000"}}"#;
        assert_eq!(
            parse_stream_info(flac),
            Some(StreamInfo {
                rate: 48000,
                channels: 1,
                duration_s: 4.0
            })
        );
        let mp3 = r#"{"streams": [{"sample_rate": "44100", "channels": 2}], "format": {"duration": "4.023129"}}"#;
        assert_eq!(
            parse_stream_info(mp3),
            Some(StreamInfo {
                rate: 44100,
                channels: 2,
                duration_s: 4.023129
            })
        );
        let both = r#"{"streams": [{"sample_rate": "44100", "channels": 2, "duration": "3.5"}], "format": {"duration": "4.0"}}"#;
        assert_eq!(
            parse_stream_info(both).unwrap().duration_s,
            3.5,
            "the stream duration wins"
        );
        let na = r#"{"streams": [{"sample_rate": "44100", "channels": 2, "duration": "N/A"}], "format": {"duration": "2.0"}}"#;
        assert_eq!(parse_stream_info(na).unwrap().duration_s, 2.0);
        for bad in [
            "garbage",
            "",
            "{}",
            r#"{"streams": []}"#,
            r#"{"streams": [{"channels": 2}], "format": {"duration": "1"}}"#,
            r#"{"streams": [{"sample_rate": "44100", "channels": 2}]}"#,
            r#"{"streams": [{"sample_rate": "0", "channels": 2, "duration": "1"}]}"#,
        ] {
            assert_eq!(parse_stream_info(bad), None, "{bad}");
        }
    }

    #[test]
    fn output_formats() {
        let si = |rate, channels| StreamInfo {
            rate,
            channels,
            duration_s: 1.0,
        };
        use ExportKind::*;
        // (source, rc5, mp3, ogg)
        let rows = [
            (si(44100, 2), (44100, 2), (44100, 2), (44100, 2)),
            (si(48000, 1), (44100, 2), (48000, 1), (48000, 1)),
            (si(22050, 2), (44100, 2), (22050, 2), (22050, 2)),
            (si(96000, 2), (44100, 2), (48000, 2), (96000, 2)),
            (si(88200, 2), (44100, 2), (44100, 2), (88200, 2)),
            (si(37800, 1), (44100, 2), (48000, 1), (37800, 1)),
            (si(44100, 6), (44100, 2), (44100, 2), (44100, 2)),
        ];
        for (src, rc5, mp3, ogg) in rows {
            assert_eq!(output_format(Rc5, &src), rc5, "{src:?}");
            assert_eq!(output_format(Mp3, &src), mp3, "{src:?}");
            assert_eq!(output_format(Ogg, &src), ogg, "{src:?}");
        }
        assert_eq!(output_format(Mp3, &si(176400, 2)), (44100, 2));
    }

    #[test]
    fn tag_lists() {
        let mut t = rec(TrackType::Stem);
        t.position = Some(3);
        t.year = Some(2021);
        let s = |v: Vec<(&str, String)>| {
            v.into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            s(tags(&t)),
            [
                "title=Stem Mix",
                "artist=The Example Band",
                "album=Export Tests",
                "track=3",
                "date=2021"
            ]
        );
        t.album = String::new();
        t.band = "  ".into();
        t.year = None;
        assert_eq!(
            s(tags(&t)),
            ["title=Stem Mix"],
            "no album: no track tag even with a position"
        );
        t.album = "A".into();
        t.position = None;
        t.title = " Tab\tbed\u{7}\n title ".into();
        assert_eq!(s(tags(&t)), ["title=Tabbed title", "album=A"]);
    }

    #[test]
    fn encoder_listing() {
        let listing = "Encoders:\n V..... = Video\n ------\n A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3) (codec mp3)\n \
                       A....D libvorbis            libvorbis (codec vorbis)\n A....D aac                 AAC (Advanced Audio Coding), like libopus\n";
        assert!(has_encoder(listing, "libmp3lame"));
        assert!(has_encoder(listing, "libvorbis"));
        assert!(
            !has_encoder(listing, "libopus"),
            "only as a word of a description"
        );
        assert!(!has_encoder(listing, "mp3"));
        assert!(!has_encoder("", "libmp3lame"));
    }

    #[test]
    fn dialog_start_folders() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let rc5 = d.path().join("rc5");
        std::fs::create_dir(&rc5).unwrap();
        assert_eq!(
            start_dir(Some(&rc5), Some(&home)),
            ("last", Some(rc5.clone()))
        );
        let gone = d.path().join("unplugged");
        assert_eq!(
            start_dir(Some(&gone), Some(&home)),
            ("home", Some(home.clone()))
        );
        assert_eq!(start_dir(None, Some(&home)), ("home", Some(home.clone())));
        assert_eq!(start_dir(Some(&gone), None), ("none", None));
        assert_eq!(start_dir(None, None), ("none", None));
        // A remembered file is not a folder either.
        let f = d.path().join("file");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(start_dir(Some(&f), Some(&home)).0, "home");
    }

    #[test]
    fn file_dialog_start_folders() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let last = d.path().join("last");
        let rec = d.path().join("rec");
        for p in [&home, &last, &rec] {
            std::fs::create_dir(p).unwrap();
        }
        let file = rec.join("a.mp3");
        assert_eq!(
            file_start_dir(Some(&file), Some(&last), Some(&home)),
            ("recorded", Some(rec.clone())),
            "the file itself need not exist"
        );
        let gone = d.path().join("gone").join("a.mp3");
        assert_eq!(
            file_start_dir(Some(&gone), Some(&last), Some(&home)),
            ("last", Some(last.clone()))
        );
        let nolast = d.path().join("nolast");
        assert_eq!(
            file_start_dir(Some(&gone), Some(&nolast), Some(&home)),
            ("home", Some(home.clone()))
        );
        assert_eq!(file_start_dir(None, Some(&last), Some(&home)).0, "last");
        assert_eq!(file_start_dir(Some(&gone), None, None), ("none", None));
    }

    #[test]
    fn quick_rc5_folder_rules() {
        let d = tempfile::tempdir().unwrap();
        let b = d.path().join("b");
        std::fs::create_dir_all(b.join("ROLAND")).unwrap();
        let bs = b.to_string_lossy().into_owned();
        assert_eq!(quick_rc5_dir(None, Some(&b)), Err("none"));
        assert_eq!(quick_rc5_dir(Some(&bs), None), Err("none"));
        assert_eq!(quick_rc5_dir(None, None), Err("none"));
        assert_eq!(quick_rc5_dir(Some("/elsewhere"), Some(&b)), Err("other"));
        assert_eq!(quick_rc5_dir(Some(&bs), Some(&b)), Ok(b.clone()));
        // A ROLAND file is not a folder; neither is a missing ROLAND.
        std::fs::remove_dir(b.join("ROLAND")).unwrap();
        assert_eq!(quick_rc5_dir(Some(&bs), Some(&b)), Err("no-roland"));
        std::fs::write(b.join("ROLAND"), "x").unwrap();
        assert_eq!(quick_rc5_dir(Some(&bs), Some(&b)), Err("no-roland"));
        std::fs::remove_dir_all(&b).unwrap();
        assert_eq!(quick_rc5_dir(Some(&bs), Some(&b)), Err("gone"));
    }

    #[test]
    fn export_kind_serde() {
        assert_eq!(serde_json::to_string(&ExportKind::Rc5).unwrap(), "\"rc5\"");
        assert_eq!(
            serde_json::from_str::<ExportKind>("\"ogg\"").unwrap(),
            ExportKind::Ogg
        );
        assert_eq!(ExportKind::Mp3.as_str(), "mp3");
    }
}
