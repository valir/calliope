//! Track metadata (`track.json`, schema version 1): parsing, validation, edits, ids and UTC
//! timestamps. Pure logic, no Tauri. See plan section 2.3.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const CURRENT_SCHEMA: u32 = 1;

const MAX_NAME: usize = 200;
const MAX_COMPOSERS: usize = 20;
const MAX_URL: usize = 2000;
const MAX_COPYRIGHT: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub band: String,
    #[serde(default)]
    pub album: String,
    pub title: String,
    #[serde(default)]
    pub composers: Vec<String>,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub copyright: Option<String>,
    pub audio: String,
    #[serde(default)]
    pub tablatures: Vec<String>,
    /// Kept verbatim as read; may be non-canonical, or empty when missing (unknown).
    #[serde(default)]
    pub imported: String,
    #[serde(default)]
    pub modified: String,
    /// Fields this build doesn't know; written back unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The user-editable fields (mirrors `TrackEdits` in the frontend).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackEdits {
    pub band: String,
    pub album: String,
    pub title: String,
    pub composers: Vec<String>,
    pub year: Option<i64>,
    pub source_url: Option<String>,
    pub copyright: Option<String>,
}

/// `^[0-9A-Za-z][0-9A-Za-z_-]{0,63}$`
pub fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    id.len() <= 64 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A fresh UUIDv7 (lower-case, hyphenated).
pub fn new_id() -> String {
    uuid::Uuid::now_v7().hyphenated().to_string()
}

/// Migration hook: upgrades a value written with schema `from` to `CURRENT_SCHEMA`.
/// Empty for v1.
pub fn migrate(value: Value, _from: u32) -> Value {
    value
}

pub fn parse(bytes: &[u8], dir_name: &str) -> Result<TrackMeta, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = value.as_object().ok_or("track.json is not a JSON object")?;
    let version = match obj.get("schema_version") {
        None => return Err("missing schema_version".into()),
        Some(v) => v
            .as_u64()
            .filter(|n| *n >= 1)
            .ok_or("schema_version must be a positive integer")?,
    };
    if version > u64::from(CURRENT_SCHEMA) {
        return Err(format!("written by a newer Calliope (schema {version})"));
    }
    let value = migrate(value, version as u32);
    let meta: TrackMeta = serde_json::from_value(value).map_err(|e| e.to_string())?;
    validate_for_read(&meta)?;
    if meta.id != dir_name {
        return Err(format!("id \"{}\" does not match folder \"{dir_name}\"", meta.id));
    }
    Ok(meta)
}

pub fn to_json_pretty(meta: &TrackMeta) -> String {
    let mut s = serde_json::to_string_pretty(meta).expect("track metadata serialises");
    s.push('\n');
    s
}

fn check_text(label: &str, s: &str, max: usize) -> Result<(), String> {
    if s.chars().count() > max {
        return Err(format!("{label} is longer than {max} characters"));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(format!("{label} contains control characters"));
    }
    if s.trim() != s {
        return Err(format!("{label} has leading or trailing spaces"));
    }
    Ok(())
}

fn check_timestamp(label: &str, s: &str) -> Result<(), String> {
    let b = s.as_bytes();
    let ok = b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        });
    if ok {
        Ok(())
    } else {
        Err(format!("{label} \"{s}\" is not an RFC 3339 UTC time (YYYY-MM-DDTHH:MM:SSZ)"))
    }
}

/// Lenient check used when loading: only things that make the track unusable or unsafe are
/// errors (bad id, empty title, bad audio or tablature file names). Cosmetic issues (padding,
/// control characters, lengths, odd years, any timestamp text) are accepted and the file is
/// never rewritten because of them.
pub fn validate_for_read(meta: &TrackMeta) -> Result<(), String> {
    if meta.schema_version != CURRENT_SCHEMA {
        return Err(format!("unsupported schema_version {}", meta.schema_version));
    }
    if !is_valid_id(&meta.id) {
        return Err(format!("invalid id \"{}\"", meta.id));
    }
    if meta.title.trim().is_empty() {
        return Err("title is empty".into());
    }
    crate::fsutil::validate_file_name(&meta.audio).map_err(|e| format!("audio: {e}"))?;
    let mut seen: Vec<String> = Vec::new();
    for t in &meta.tablatures {
        crate::fsutil::validate_file_name(t).map_err(|e| format!("tablature: {e}"))?;
        let lower = t.to_lowercase();
        if lower == meta.audio.to_lowercase() {
            return Err(format!("tablature \"{t}\" is the audio file"));
        }
        if seen.contains(&lower) {
            return Err(format!("tablature \"{t}\" is listed twice"));
        }
        seen.push(lower);
    }
    Ok(())
}

/// Strict rules for the user-editable fields (applied when writing).
fn validate_edit_fields(meta: &TrackMeta) -> Result<(), String> {
    check_text("band", &meta.band, MAX_NAME)?;
    check_text("album", &meta.album, MAX_NAME)?;
    if meta.title.trim().is_empty() {
        return Err("title is empty".into());
    }
    check_text("title", &meta.title, MAX_NAME)?;
    if meta.composers.len() > MAX_COMPOSERS {
        return Err(format!("more than {MAX_COMPOSERS} composers"));
    }
    for c in &meta.composers {
        if c.trim().is_empty() {
            return Err("a composer name is empty".into());
        }
        check_text("composer", c, MAX_NAME)?;
    }
    if let Some(y) = meta.year {
        if !(1..=9999).contains(&y) {
            return Err(format!("year {y} is outside 1..9999"));
        }
    }
    if let Some(u) = &meta.source_url {
        check_text("source_url", u, MAX_URL)?;
    }
    if let Some(c) = &meta.copyright {
        check_text("copyright", c, MAX_COPYRIGHT)?;
    }
    Ok(())
}

/// Full strict check for a track Calliope writes from scratch (canonical timestamps too).
/// Not used for tracks loaded from disk: their `imported` is preserved as read.
pub fn validate_for_write(meta: &TrackMeta) -> Result<(), String> {
    validate_for_read(meta)?;
    validate_edit_fields(meta)?;
    check_timestamp("imported", &meta.imported)?;
    check_timestamp("modified", &meta.modified)
}

fn opt_trim(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Applies user edits (trimmed; empty optional strings become `None`) and validates the
/// edited fields strictly. Timestamps are left exactly as loaded. On error `meta` is left unchanged. Does not touch `modified`.
pub fn apply_edits(meta: &mut TrackMeta, edits: TrackEdits) -> Result<(), String> {
    let mut next = meta.clone();
    next.band = edits.band.trim().to_string();
    next.album = edits.album.trim().to_string();
    next.title = edits.title.trim().to_string();
    next.composers = edits.composers.iter().map(|c| c.trim().to_string()).collect();
    next.year = edits.year;
    next.source_url = opt_trim(edits.source_url);
    next.copyright = opt_trim(edits.copyright);
    validate_edit_fields(&next)?;
    validate_for_read(&next)?;
    *meta = next;
    Ok(())
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (days-to-civil conversion).
pub fn rfc3339_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_utc(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
  "schema_version": 1,
  "id": "0199b3f2-6c1e-7a3b-9d2e-4f5a6b7c8d9e",
  "band": "Amber Fields",
  "album": "Northern Roads",
  "title": "Slow Burn",
  "composers": ["Ann Example", "Bo Sample"],
  "year": 2019,
  "source_url": "https://example.org/slow-burn",
  "copyright": "© 2019 Amber Fields",
  "audio": "backing.mp3",
  "tablatures": ["slow-burn.gp5", "slow-burn-solo.gp"],
  "imported": "2026-10-05T14:03:22Z",
  "modified": "2026-10-05T14:10:00Z"
}"#;
    const ID: &str = "0199b3f2-6c1e-7a3b-9d2e-4f5a6b7c8d9e";

    fn example() -> TrackMeta {
        parse(EXAMPLE.as_bytes(), ID).unwrap()
    }

    fn edits_of(m: &TrackMeta) -> TrackEdits {
        TrackEdits {
            band: m.band.clone(),
            album: m.album.clone(),
            title: m.title.clone(),
            composers: m.composers.clone(),
            year: m.year,
            source_url: m.source_url.clone(),
            copyright: m.copyright.clone(),
        }
    }

    #[test]
    fn round_trip_of_the_example() {
        let m = example();
        assert_eq!(m.title, "Slow Burn");
        assert_eq!(m.year, Some(2019));
        let back = to_json_pretty(&m);
        let a: Value = serde_json::from_str(EXAMPLE).unwrap();
        let b: Value = serde_json::from_str(&back).unwrap();
        assert_eq!(a, b);
        assert_eq!(parse(back.as_bytes(), ID).unwrap(), m);
    }

    #[test]
    fn unknown_fields_survive() {
        let text = EXAMPLE.replace("\"year\"", "\"future\": {\"a\": [1, 2]},\n  \"year\"");
        let m = parse(text.as_bytes(), ID).unwrap();
        assert!(m.extra.contains_key("future"));
        let back: Value = serde_json::from_str(&to_json_pretty(&m)).unwrap();
        assert_eq!(back["future"], serde_json::json!({"a": [1, 2]}));
    }

    #[test]
    fn schema_errors() {
        let newer = EXAMPLE.replace("\"schema_version\": 1", "\"schema_version\": 2");
        assert_eq!(
            parse(newer.as_bytes(), ID).unwrap_err(),
            "written by a newer Calliope (schema 2)"
        );
        let missing = EXAMPLE.replace("\"schema_version\": 1,", "");
        assert_eq!(parse(missing.as_bytes(), ID).unwrap_err(), "missing schema_version");
        assert!(parse(b"[]", ID).is_err());
        assert!(parse(b"{nope", ID).is_err());
    }

    #[test]
    fn migrate_hook_is_identity_for_v1() {
        let v: Value = serde_json::from_str(EXAMPLE).unwrap();
        assert_eq!(migrate(v.clone(), 1), v);
    }

    #[test]
    fn wrong_type_is_an_error() {
        let text = EXAMPLE.replace("\"year\": 2019", "\"year\": \"1999\"");
        assert!(parse(text.as_bytes(), ID).is_err());
    }

    #[test]
    fn id_must_match_folder() {
        let e = parse(EXAMPLE.as_bytes(), "other").unwrap_err();
        assert_eq!(e, format!("id \"{ID}\" does not match folder \"other\""));
    }

    #[test]
    fn write_validation_rules() {
        let bad: [fn(&mut TrackMeta); 8] = [
            |m| m.title = "  ".into(),
            |m| m.composers = (0..21).map(|i| format!("c{i}")).collect(),
            |m| m.year = Some(0),
            |m| m.tablatures = vec!["A.gp5".into(), "a.GP5".into()],
            |m| m.tablatures = vec!["BACKING.mp3".into()],
            |m| m.audio = "../x.mp3".into(),
            |m| m.modified = "yesterday".into(),
            |m| m.band = " x ".into(),
        ];
        assert!(validate_for_write(&example()).is_ok());
        for f in bad {
            let mut m = example();
            f(&mut m);
            assert!(validate_for_write(&m).is_err());
        }
    }

    #[test]
    fn read_is_lenient_about_cosmetics() {
        let text = EXAMPLE
            .replace("\"Amber Fields\"", "\"  Amber Fields \"")
            .replace("2026-10-05T14:03:22Z", "2026-10-05T16:03:22.5+02:00")
            .replace("\"year\": 2019", "\"year\": 20190")
            .replace("2026-10-05T14:10:00Z", "whenever");
        let m = parse(text.as_bytes(), ID).unwrap();
        assert_eq!(m.band, "  Amber Fields ");
        assert_eq!(m.imported, "2026-10-05T16:03:22.5+02:00");
        assert_eq!(m.modified, "whenever");
        let no_times = EXAMPLE
            .replace(",\n  \"imported\": \"2026-10-05T14:03:22Z\"", "")
            .replace(",\n  \"modified\": \"2026-10-05T14:10:00Z\"", "");
        assert_eq!(parse(no_times.as_bytes(), ID).unwrap().imported, "");
    }

    #[test]
    fn read_still_rejects_unusable_tracks() {
        for (from, to) in [
            ("\"title\": \"Slow Burn\"", "\"title\": \"  \""),
            ("\"backing.mp3\"", "\"../backing.mp3\""),
            ("\"slow-burn.gp5\"", "\"a/b.gp5\""),
        ] {
            assert!(parse(EXAMPLE.replace(from, to).as_bytes(), ID).is_err(), "{to}");
        }
    }

    #[test]
    fn save_after_lenient_load_keeps_imported() {
        let text = EXAMPLE.replace("2026-10-05T14:03:22Z", "2026-10-05T16:03:22+02:00");
        let mut m = parse(text.as_bytes(), ID).unwrap();
        let mut e = edits_of(&m);
        e.band = "  New Band ".into();
        apply_edits(&mut m, e).unwrap();
        assert_eq!(m.band, "New Band");
        assert_eq!(m.imported, "2026-10-05T16:03:22+02:00");
        let mut e = edits_of(&m);
        e.album = "a\u{7}b".into();
        assert!(apply_edits(&mut m, e).is_err());
    }

    #[test]
    fn apply_edits_trims_and_clears() {
        let mut m = example();
        let mut e = edits_of(&m);
        e.title = "  New  ".into();
        e.band = " ".into();
        e.source_url = Some("  ".into());
        e.copyright = None;
        e.composers = vec![" Z ".into()];
        apply_edits(&mut m, e).unwrap();
        assert_eq!(m.title, "New");
        assert_eq!(m.band, "");
        assert_eq!(m.source_url, None);
        assert_eq!(m.copyright, None);
        assert_eq!(m.composers, vec!["Z"]);
    }

    #[test]
    fn apply_edits_rejects_and_keeps_meta() {
        let mut m = example();
        let before = m.clone();
        let mut e = edits_of(&m);
        e.title = "  ".into();
        assert!(apply_edits(&mut m, e).is_err());
        assert_eq!(m, before);
        let mut e = edits_of(&m);
        e.composers = vec!["  ".into()];
        assert!(apply_edits(&mut m, e).is_err());
    }

    #[test]
    fn ids() {
        assert!(is_valid_id(ID));
        assert!(is_valid_id("a"));
        assert!(is_valid_id("Track_1-b"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("-a"));
        assert!(!is_valid_id("a b"));
        assert!(!is_valid_id("a/b"));
        assert!(!is_valid_id(&"a".repeat(65)));
        assert!(is_valid_id(&"a".repeat(64)));
        let (a, b) = (new_id(), new_id());
        assert!(is_valid_id(&a));
        assert_ne!(a, b);
    }

    #[test]
    fn timestamps() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_790_000_000), "2026-09-21T14:13:20Z");
        let now = now_rfc3339();
        assert!(check_timestamp("now", &now).is_ok());
    }
}
