//! Player settings of one track, stored as `player.json` next to `track.json`. Pure logic and
//! file IO, no Tauri. Reading is lenient (bad parts are dropped, unknown fields are kept) and a
//! newer schema is read-only. Writing never creates the track folder.

use crate::fsutil;
use crate::tempo_map::{self, Boundary, Segment, TempoMap, MAX_BPM, MIN_BPM};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

pub(crate) const PLAYER_FILE: &str = "player.json";
pub(crate) const SCHEMA: u32 = 2;
pub(crate) const MAX_LOOPS: usize = 99;
pub(crate) const MIN_SPEED: f64 = 0.5;
pub(crate) const MAX_SPEED: f64 = 1.5;
const MAX_BEATS_PER_BAR: u64 = 32;

pub(crate) const NOTE_UNREADABLE: &str =
    "Player settings in player.json could not be read; they will be replaced when you change something";
pub(crate) const NOTE_NEWER: &str =
    "player.json was written by a newer version; the player settings will not be saved";
pub(crate) const END_BEFORE_START: &str = "The end must be after the start";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Metronome {
    pub on: bool,
    pub count_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LoopRec {
    pub id: String,
    pub start: Boundary,
    pub end: Option<Boundary>,
    pub repeat: bool,
    pub count_in: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl LoopRec {
    pub fn is_complete(&self) -> bool {
        self.end.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlayerState {
    pub schema_version: u32,
    pub source: Option<String>,
    pub position_ms: u64,
    pub tempo: TempoMap,
    pub speed: f64,
    pub metronome: Metronome,
    pub loops: Vec<LoopRec>,
    pub current_loop: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    /// A newer schema was read: the state is never written.
    #[serde(skip)]
    pub read_only: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            schema_version: SCHEMA,
            source: None,
            position_ms: 0,
            tempo: TempoMap::default(),
            speed: 1.0,
            metronome: Metronome { on: false, count_in: false },
            loops: Vec::new(),
            current_loop: None,
            extra: Map::new(),
            read_only: false,
        }
    }
}

/// Which loop boundary a mark sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    Start,
    End,
}

// ---- reading and writing ----

fn parse_boundary(v: &Value) -> Option<Boundary> {
    match serde_json::from_value::<Boundary>(v.clone()).ok()? {
        Boundary::Ms { ms } if !ms.is_finite() => None,
        b => Some(b),
    }
}

fn parse_loop(v: &Value) -> Option<LoopRec> {
    let o = v.as_object()?;
    let id = o.get("id")?.as_str().filter(|s| !s.is_empty())?.to_string();
    let start = parse_boundary(o.get("start")?)?;
    let end = o.get("end").and_then(parse_boundary);
    let flag = |k: &str, d: bool| o.get(k).and_then(Value::as_bool).unwrap_or(d);
    let mut extra = o.clone();
    for k in ["id", "start", "end", "repeat", "count_in"] {
        extra.remove(k);
    }
    Some(LoopRec { id, start, end, repeat: flag("repeat", true), count_in: flag("count_in", false), extra })
}

/// Largest accepted tempo-segment start (about 11 days); anything beyond is a damaged file.
const MAX_SEGMENT_MS: f64 = 1.0e9;

fn parse_tempo(v: Option<&Value>) -> TempoMap {
    let mut map = TempoMap::default();
    let Some(o) = v.and_then(Value::as_object) else { return map };
    if let Some(n) = o.get("beats_per_bar").and_then(Value::as_u64).filter(|n| (1..=MAX_BEATS_PER_BAR).contains(n)) {
        map.beats_per_bar = n as u32;
    }
    if let Some(a) = o.get("segments").and_then(Value::as_array) {
        for s in a {
            let start_ms = s.get("start_ms").and_then(Value::as_f64).filter(|m| m.abs() <= MAX_SEGMENT_MS);
            let bpm = s.get("bpm").and_then(Value::as_f64).filter(|b| (MIN_BPM..=MAX_BPM).contains(b));
            let detected_ms = s.get("detected_ms").and_then(Value::as_f64).filter(|m| m.is_finite() && m.abs() <= MAX_SEGMENT_MS);
            if let (Some(start_ms), Some(bpm)) = (start_ms, bpm) {
                map.segments.push(Segment { start_ms, bpm, detected_ms });
            }
        }
        map.segments.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        map.segments.dedup_by(|b, a| a.start_ms == b.start_ms);
        if let Some(first) = map.segments.first_mut() {
            first.detected_ms = None;
        }
    }
    map
}

/// Builds a state from a parsed value, dropping bad parts. Never fails.
fn from_value(v: &Value) -> PlayerState {
    let mut st = PlayerState::default();
    let Some(o) = v.as_object() else { return st };
    st.source = o.get("source").and_then(Value::as_str).map(str::to_string);
    st.position_ms = o
        .get("position_ms")
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite())
        .map_or(0, |p| p.round().max(0.0) as u64);
    st.tempo = parse_tempo(o.get("tempo"));
    st.speed = o
        .get("speed")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite())
        .map_or(1.0, |s| s.clamp(MIN_SPEED, MAX_SPEED));
    if let Some(m) = o.get("metronome").and_then(Value::as_object) {
        st.metronome.on = m.get("on").and_then(Value::as_bool).unwrap_or(false);
        st.metronome.count_in = m.get("count_in").and_then(Value::as_bool).unwrap_or(false);
    }
    if let Some(a) = o.get("loops").and_then(Value::as_array) {
        for l in a.iter().filter_map(parse_loop) {
            if st.loops.len() < MAX_LOOPS && !st.loops.iter().any(|x| x.id == l.id) {
                st.loops.push(l);
            }
        }
    }
    st.current_loop = o
        .get("current_loop")
        .and_then(Value::as_str)
        .filter(|id| st.loops.iter().any(|l| l.id == *id))
        .map(str::to_string);
    st.extra = o.clone();
    for k in
        ["schema_version", "source", "position_ms", "tempo", "speed", "metronome", "loops", "current_loop"]
    {
        st.extra.remove(k);
    }
    st.sort_loops();
    // Loops without a (valid) current one: the first loop becomes current.
    if st.current_loop.is_none() {
        st.current_loop = st.loops.first().map(|l| l.id.clone());
    }
    st
}

/// Reads `<dir>/player.json`. Returns the state, an optional inline note and whether the state may
/// be written. A missing file gives defaults and no note.
pub(crate) fn load(dir: &Path) -> (PlayerState, Option<String>, bool) {
    let bytes = match std::fs::read(dir.join(PLAYER_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (PlayerState::default(), None, true),
        Err(_) => return (PlayerState::default(), Some(NOTE_UNREADABLE.into()), true),
    };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        return (PlayerState::default(), Some(NOTE_UNREADABLE.into()), true);
    };
    if !v.is_object() {
        return (PlayerState::default(), Some(NOTE_UNREADABLE.into()), true);
    }
    match v.get("schema_version").and_then(Value::as_u64) {
        Some(n) if n > u64::from(SCHEMA) => {
            let st = PlayerState { read_only: true, ..PlayerState::default() };
            return (st, Some(NOTE_NEWER.into()), false);
        }
        Some(_) => {}
        None => return (PlayerState::default(), Some(NOTE_UNREADABLE.into()), true),
    }
    (from_value(&v), None, true)
}

/// Writes `<dir>/player.json` atomically. Refuses a read-only state and never creates `dir`.
pub(crate) fn save(dir: &Path, state: &PlayerState) -> Result<(), String> {
    if state.read_only {
        return Err("player.json was written by a newer version and is not overwritten".into());
    }
    if !dir.is_dir() {
        return Err(format!("the track folder {} does not exist", dir.display()));
    }
    let mut s = state.clone();
    s.schema_version = SCHEMA;
    let mut text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    text.push('\n');
    fsutil::write_atomic(&dir.join(PLAYER_FILE), text.as_bytes())
        .map_err(|e| format!("cannot write {PLAYER_FILE}: {e}"))
}

/// The default source rule (plan 2.2): the saved key if it still exists, else the first
/// `backing:*`, else `audio`, else the first `stem:*`. `original` is never chosen automatically.
pub fn choose_source<S: AsRef<str>>(saved: Option<&str>, keys: &[S]) -> Option<String> {
    let has = |k: &str| keys.iter().any(|x| x.as_ref() == k);
    if let Some(s) = saved.filter(|s| has(s)) {
        return Some(s.to_string());
    }
    let first = |prefix: &str| keys.iter().map(AsRef::as_ref).find(|k| k.starts_with(prefix));
    first("backing:")
        .or_else(|| keys.iter().map(AsRef::as_ref).find(|k| *k == "audio"))
        .or_else(|| first("stem:"))
        .map(str::to_string)
}

// ---- loops ----

impl PlayerState {
    fn boundary_at(&self, position_ms: f64) -> Boundary {
        if self.tempo.is_empty() {
            Boundary::Ms { ms: position_ms }
        } else {
            Boundary::Tick { tick: self.tempo.tick_near(position_ms) }
        }
    }

    /// Position of a boundary in ms, for ordering.
    fn ms_of(&self, b: Boundary) -> f64 {
        match b {
            Boundary::Ms { ms } => ms,
            Boundary::Tick { tick } => self.tempo.ms_at(tick),
        }
    }

    fn before(&self, a: Boundary, b: Boundary) -> bool {
        match (a, b) {
            (Boundary::Tick { tick: x }, Boundary::Tick { tick: y }) => x < y,
            _ => self.ms_of(a) < self.ms_of(b),
        }
    }

    fn sort_loops(&mut self) {
        let keys: Vec<(String, f64)> = self.loops.iter().map(|l| (l.id.clone(), self.ms_of(l.start))).collect();
        self.loops.sort_by(|a, b| {
            let ka = keys.iter().find(|k| k.0 == a.id).map_or(0.0, |k| k.1);
            let kb = keys.iter().find(|k| k.0 == b.id).map_or(0.0, |k| k.1);
            ka.total_cmp(&kb)
        });
    }

    pub fn current(&self) -> Option<&LoopRec> {
        let id = self.current_loop.as_deref()?;
        self.loops.iter().find(|l| l.id == id)
    }

    fn current_mut(&mut self) -> Result<&mut LoopRec, String> {
        let id = self.current_loop.clone().ok_or("There is no loop selected")?;
        self.loops.iter_mut().find(|l| l.id == id).ok_or_else(|| "There is no loop selected".to_string())
    }

    /// Adds a loop starting at the position, with no end; it becomes current. Returns its id.
    pub fn add(&mut self, position_ms: f64) -> Result<String, String> {
        if self.loops.len() >= MAX_LOOPS {
            return Err(format!("At most {MAX_LOOPS} loops"));
        }
        let n = self
            .loops
            .iter()
            .filter_map(|l| l.id.strip_prefix("loop-").and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0)
            + 1;
        let id = format!("loop-{n}");
        let start = self.boundary_at(position_ms);
        self.loops.push(LoopRec {
            id: id.clone(),
            start,
            end: None,
            repeat: true,
            count_in: false,
            extra: Map::new(),
        });
        self.sort_loops();
        self.current_loop = Some(id.clone());
        Ok(id)
    }

    /// Removes the current loop; the next one (or the previous, if it was last) becomes current.
    pub fn remove(&mut self) -> Result<(), String> {
        let id = self.current_loop.clone().ok_or("There is no loop selected")?;
        let i = self.loops.iter().position(|l| l.id == id).ok_or("There is no loop selected")?;
        self.loops.remove(i);
        self.current_loop = self.loops.get(i.min(self.loops.len().saturating_sub(1))).map(|l| l.id.clone());
        Ok(())
    }

    pub fn select(&mut self, id: &str) -> Result<(), String> {
        if !self.loops.iter().any(|l| l.id == id) {
            return Err(format!("There is no loop {id}"));
        }
        self.current_loop = Some(id.to_string());
        Ok(())
    }

    /// `<` and `>` of the loop section (plan 2.4). On refusal nothing changes.
    pub fn mark(&mut self, which: Mark, position_ms: f64) -> Result<(), String> {
        let b = self.boundary_at(position_ms);
        let start = self.current_mut()?.start;
        match which {
            Mark::End => {
                if !self.before(start, b) {
                    return Err(END_BEFORE_START.into());
                }
                self.current_mut()?.end = Some(b);
            }
            Mark::Start => {
                let end = self.current_mut()?.end;
                let l = self.current_mut()?;
                l.start = b;
                if end.is_some_and(|e| !self_before(b, e, &self.tempo)) {
                    self.current_mut()?.end = None;
                }
                self.sort_loops();
            }
        }
        Ok(())
    }

    /// Typed boundary value (deliberate: an invalid case is refused and changes nothing).
    pub fn set(&mut self, which: Mark, value: Boundary) -> Result<(), String> {
        let ok_mode = match value {
            Boundary::Tick { .. } => !self.tempo.is_empty(),
            Boundary::Ms { ms } => self.tempo.is_empty() && ms.is_finite(),
        };
        if !ok_mode {
            return Err("The value does not fit the current mode".into());
        }
        let (start, end) = {
            let l = self.current_mut()?;
            (l.start, l.end)
        };
        match which {
            Mark::Start => {
                if end.is_some_and(|e| !self.before(value, e)) {
                    return Err("The start must be before the end".into());
                }
                self.current_mut()?.start = value;
                self.sort_loops();
            }
            Mark::End => {
                if !self.before(start, value) {
                    return Err(END_BEFORE_START.into());
                }
                self.current_mut()?.end = Some(value);
            }
        }
        Ok(())
    }

    /// Sets `repeat` and/or `count_in` of the current loop; refused on an incomplete loop.
    pub fn flags(&mut self, repeat: Option<bool>, count_in: Option<bool>) -> Result<(), String> {
        let l = self.current_mut()?;
        if !l.is_complete() {
            return Err("Set the loop end first".into());
        }
        if let Some(r) = repeat {
            l.repeat = r;
        }
        if let Some(c) = count_in {
            l.count_in = c;
        }
        Ok(())
    }

    /// Replaces the tempo map and re-expresses every loop boundary, keeping its time.
    pub fn apply_tempo_change(&mut self, new_map: TempoMap) {
        let old = std::mem::replace(&mut self.tempo, new_map);
        for l in &mut self.loops {
            l.start = tempo_map::resnap(l.start, &old, &self.tempo);
            l.end = l.end.map(|e| tempo_map::resnap(e, &old, &self.tempo));
        }
        self.sort_loops();
    }
}

fn self_before(a: Boundary, b: Boundary, tempo: &TempoMap) -> bool {
    match (a, b) {
        (Boundary::Tick { tick: x }, Boundary::Tick { tick: y }) => x < y,
        (a, b) => ms_in(a, tempo) < ms_in(b, tempo),
    }
}

fn ms_in(b: Boundary, tempo: &TempoMap) -> f64 {
    match b {
        Boundary::Ms { ms } => ms,
        Boundary::Tick { tick } => tempo.ms_at(tick),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const WITH_TEMPO: &str = include_str!(
        "../tests/fixtures/library-player/tracks/0199c0a0-0000-7000-8000-000000000304/player.json"
    );

    fn dir_with(text: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(PLAYER_FILE), text).unwrap();
        d
    }

    fn bpm_state() -> PlayerState {
        let tempo = TempoMap { beats_per_bar: 4, segments: vec![Segment { start_ms: 500.0, bpm: 120.0, detected_ms: None }] };
        PlayerState { tempo, ..PlayerState::default() }
    }

    #[test]
    fn round_trip_with_tempo() {
        let mut v: Value = serde_json::from_str(WITH_TEMPO).unwrap();
        v["future"] = json!({"a": 1});
        v["loops"][0]["note"] = json!("x");
        let d = dir_with(&v.to_string());
        let (st, note, writable) = load(d.path());
        assert!(note.is_none() && writable);
        assert_eq!(st.position_ms, 4000);
        assert_eq!(st.loops.len(), 3);
        assert_eq!(st.loops[2].id, "loop-3");
        assert_eq!(st.loops[2].end, None);
        assert_eq!(st.current_loop.as_deref(), Some("loop-1"));
        save(d.path(), &st).unwrap();
        let back: Value = serde_json::from_slice(&std::fs::read(d.path().join(PLAYER_FILE)).unwrap()).unwrap();
        // the v1 fixture reads with the same map and loops and is written back as schema 2
        assert_eq!(back["schema_version"], 2);
        v["schema_version"] = json!(2);
        assert_eq!(back, v);
        assert!(back["loops"][2]["end"].is_null());
        assert_eq!(back["position_ms"], 4000);
    }

    #[test]
    fn detected_ms_round_trips_and_bad_values_are_dropped() {
        let d = dir_with(
            &json!({"schema_version": 2, "tempo": {"segments": [
                {"start_ms": 0.0, "bpm": 120.0, "detected_ms": 5.0},
                {"start_ms": 6500.0, "bpm": 90.0, "detected_ms": 6500.0},
                {"start_ms": 9000.0, "bpm": 100.0, "detected_ms": 1e12},
                {"start_ms": 12000.0, "bpm": 110.0, "detected_ms": "x"}]}})
            .to_string(),
        );
        let (st, note, _) = load(d.path());
        assert!(note.is_none());
        let det: Vec<_> = st.tempo.segments.iter().map(|s| s.detected_ms).collect();
        assert_eq!(det, [None, Some(6500.0), None, None]);
        assert_eq!(st.tempo.segments.len(), 4);
        save(d.path(), &st).unwrap();
        let (back, _, _) = load(d.path());
        assert_eq!(back.tempo, st.tempo);
        let v: Value = serde_json::from_slice(&std::fs::read(d.path().join(PLAYER_FILE)).unwrap()).unwrap();
        assert_eq!(v["tempo"]["segments"][1]["detected_ms"], 6500.0);
        assert!(v["tempo"]["segments"][0].get("detected_ms").is_none());
    }

    #[test]
    fn lenient_read() {
        let d = dir_with("{ not json");
        let (st, note, w) = load(d.path());
        assert_eq!(st, PlayerState::default());
        assert!(note.unwrap().contains("could not be read") && w);

        let d = tempfile::tempdir().unwrap();
        assert_eq!(load(d.path()), (PlayerState::default(), None, true));

        let d = dir_with(
            &json!({"schema_version": 1, "speed": 9.0, "position_ms": -5,
              "tempo": {"beats_per_bar": 3, "segments": [{"start_ms": 0.0, "bpm": 500.0}, {"start_ms": 10.0, "bpm": 100.0}]},
              "loops": [
                {"id": "a", "start": {"tick": 5}, "end": {"tick": "x"}},
                {"id": "b", "start": "bad", "end": {"tick": 9}},
                {"start": {"tick": 1}},
                {"id": "c", "start": {"tick": 1}, "end": {"tick": 9}}],
              "current_loop": "zzz"})
            .to_string(),
        );
        let (st, note, _) = load(d.path());
        assert!(note.is_none());
        assert_eq!(st.speed, MAX_SPEED);
        assert_eq!(st.position_ms, 0);
        assert_eq!(st.tempo.segments, vec![Segment { start_ms: 10.0, bpm: 100.0, detected_ms: None }]);
        assert_eq!(st.tempo.beats_per_bar, 3);
        let ids: Vec<_> = st.loops.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["c", "a"]);
        assert_eq!(st.loops[1].end, None);
        assert_eq!(st.current_loop.as_deref(), Some("c")); // stale id: the first loop
    }

    #[test]
    fn newer_schema_is_read_only_and_folders_are_not_created() {
        let text = r#"{"schema_version": 3, "source": "audio"}"#;
        let d = dir_with(text);
        let (st, note, w) = load(d.path());
        assert!(!w && note.is_some() && st.read_only);
        assert!(save(d.path(), &st).is_err());
        assert_eq!(std::fs::read_to_string(d.path().join(PLAYER_FILE)).unwrap(), text);

        let gone = tempfile::tempdir().unwrap().path().join("deleted");
        assert!(save(&gone, &PlayerState::default()).is_err());
        assert!(!gone.exists());
    }

    #[test]
    fn loop_rules_bpm_mode() {
        let mut s = bpm_state();
        s.add(3000.0).unwrap();
        assert_eq!(s.loops[0].start, Boundary::Tick { tick: 80 });
        assert_eq!(s.loops[0].end, None);
        assert!(s.flags(Some(false), None).is_err());
        s.mark(Mark::End, 9000.0).unwrap();
        assert_eq!(s.loops[0].end, Some(Boundary::Tick { tick: 272 }));

        let before = s.clone();
        assert_eq!(s.mark(Mark::End, 2000.0).unwrap_err(), "The end must be after the start");
        assert_eq!(s, before);

        s.mark(Mark::Start, 4000.0).unwrap();
        assert_eq!(s.loops[0].start, Boundary::Tick { tick: 112 });
        assert_eq!(s.loops[0].end, Some(Boundary::Tick { tick: 272 }));
        s.mark(Mark::Start, 3000.0).unwrap();
        assert_eq!(s.loops[0].start, Boundary::Tick { tick: 80 });
        assert_eq!(s.loops[0].end, Some(Boundary::Tick { tick: 272 }));

        s.mark(Mark::Start, 10000.0).unwrap();
        assert_eq!(s.loops[0].start, Boundary::Tick { tick: 304 });
        assert_eq!(s.loops[0].end, None);

        s.mark(Mark::Start, 3000.0).unwrap();
        s.mark(Mark::End, 9000.0).unwrap();
        let before = s.clone();
        assert!(s.set(Mark::Start, Boundary::Tick { tick: 272 }).is_err());
        assert!(s.set(Mark::Start, Boundary::Tick { tick: 300 }).is_err());
        assert!(s.set(Mark::End, Boundary::Tick { tick: 80 }).is_err());
        assert!(s.set(Mark::End, Boundary::Ms { ms: 5.0 }).is_err());
        assert_eq!(s, before);
        s.set(Mark::End, Boundary::Tick { tick: 400 }).unwrap();
        s.flags(Some(false), Some(true)).unwrap();
        assert!(!s.loops[0].repeat && s.loops[0].count_in);
    }

    #[test]
    fn timestamp_mode_and_tempo_change() {
        let mut s = PlayerState::default();
        s.add(1000.0).unwrap();
        s.mark(Mark::End, 2500.0).unwrap();
        assert_eq!(s.loops[0].end, Some(Boundary::Ms { ms: 2500.0 }));
        let mut m = TempoMap::default();
        m.set_bpm_at(500.0, 120.0).unwrap();
        s.apply_tempo_change(m);
        assert_eq!(s.loops[0].start, Boundary::Tick { tick: 16 });
        assert_eq!(s.loops[0].end, Some(Boundary::Tick { tick: 64 }));
        s.apply_tempo_change(TempoMap::default());
        assert_eq!(s.loops[0].start, Boundary::Ms { ms: 1000.0 });
        assert_eq!(s.loops[0].end, Some(Boundary::Ms { ms: 2500.0 }));
    }

    #[test]
    fn order_and_current_survive() {
        let mut s = bpm_state();
        let a = s.add(8000.0).unwrap();
        let b = s.add(2000.0).unwrap();
        let c = s.add(5000.0).unwrap();
        let ids: Vec<_> = s.loops.iter().map(|l| l.id.clone()).collect();
        assert_eq!(ids, [b.clone(), c.clone(), a.clone()]);
        assert_eq!(s.current_loop.as_deref(), Some(c.as_str()));
        s.mark(Mark::Start, 20000.0).unwrap();
        let ids: Vec<_> = s.loops.iter().map(|l| l.id.clone()).collect();
        assert_eq!(ids, [b.clone(), a.clone(), c.clone()]);
        assert_eq!(s.current().unwrap().id, c);
        s.remove().unwrap();
        assert_eq!(s.current_loop.as_deref(), Some(a.as_str()));
        s.select(&b).unwrap();
        s.remove().unwrap();
        assert_eq!(s.current_loop.as_deref(), Some(a.as_str()));
        s.remove().unwrap();
        assert_eq!(s.current_loop, None);
        assert!(s.remove().is_err());
    }

    #[test]
    fn max_loops() {
        let mut s = PlayerState::default();
        for i in 0..MAX_LOOPS {
            s.add(i as f64).unwrap();
        }
        assert!(s.add(1e6).is_err());
        assert_eq!(s.loops.len(), MAX_LOOPS);
    }

    #[test]
    fn source_choice() {
        let k = ["original", "stem:guitar", "backing:backing", "audio", "stem:vocals"];
        assert_eq!(choose_source(None, &k).as_deref(), Some("backing:backing"));
        assert_eq!(choose_source(Some("stem:vocals"), &k).as_deref(), Some("stem:vocals"));
        assert_eq!(choose_source(Some("stem:gone"), &k).as_deref(), Some("backing:backing"));
        assert_eq!(choose_source(None, &["original", "audio", "stem:a"]).as_deref(), Some("audio"));
        assert_eq!(choose_source(None, &["original", "stem:a", "stem:b"]).as_deref(), Some("stem:a"));
        assert_eq!(choose_source(None, &["original"]), None);
    }
}
