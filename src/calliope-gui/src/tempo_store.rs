//! Where the tempo map of a track is read and replaced (plan 2.2): `player.json`, shared by the
//! Editor and the Player. `FileTempoStore` is the file part; the app's `PlayerTempoStore`
//! (`player_ipc.rs`) first releases the Player's session for the track.

use std::path::Path;

use crate::player_state;
use crate::tempo_map::TempoMap;

pub const NEWER: &str = "player.json was written by a newer version; the tempo was not saved";

/// The tempo map of a track. Implementations must make `replace` atomic with respect to the
/// Player (it never overwrites the Player's loops or position, and the Player never writes
/// the old map back).
pub trait TempoStore: Send + Sync {
    /// The current map (empty when there is none).
    fn load(&self, id: &str, dir: &Path) -> Result<TempoMap, String>;
    /// Replaces the map by `f(old)`; loops keep their time, `speed` becomes 1.0, every other
    /// field is kept. Returns the new map.
    fn replace(
        &self,
        id: &str,
        dir: &Path,
        f: &dyn Fn(&TempoMap) -> Result<TempoMap, String>,
    ) -> Result<TempoMap, String>;
}

/// Reads and writes `<dir>/player.json` only. Never creates the track folder.
pub struct FileTempoStore;

impl TempoStore for FileTempoStore {
    fn load(&self, _id: &str, dir: &Path) -> Result<TempoMap, String> {
        Ok(player_state::load(dir).0.tempo)
    }

    fn replace(
        &self,
        _id: &str,
        dir: &Path,
        f: &dyn Fn(&TempoMap) -> Result<TempoMap, String>,
    ) -> Result<TempoMap, String> {
        let (mut state, _note, writable) = player_state::load(dir);
        if !writable || state.read_only {
            return Err(NEWER.into());
        }
        let new = f(&state.tempo)?;
        state.apply_tempo_change(new.clone());
        state.speed = 1.0;
        player_state::save(dir, &state).map_err(|e| format!("Could not save the tempo: {e}"))?;
        Ok(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo_map::{Boundary, SectionMeta, Segment};
    use serde_json::json;

    fn map(bpm: f64) -> TempoMap {
        TempoMap { beats_per_bar: 4, segments: vec![Segment { start_ms: 500.0, bpm, detected_ms: None, on_beat: false, meta: SectionMeta::default() }] }
    }

    fn write(dir: &Path, v: serde_json::Value) {
        std::fs::write(dir.join("player.json"), serde_json::to_vec(&v).unwrap()).unwrap();
    }

    fn read(dir: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(dir.join("player.json")).unwrap()).unwrap()
    }

    #[test]
    fn replace_keeps_loop_times_unknown_fields_and_resets_speed() {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            json!({"schema_version": 2, "position_ms": 1234, "speed": 0.8, "future": {"a": 1},
                   "tempo": {"beats_per_bar": 3, "segments": [{"start_ms": 500.0, "bpm": 120.0}]},
                   "loops": [{"id": "l1", "start": {"tick": 16}, "end": {"tick": 48}, "repeat": true, "count_in": false}]}),
        );
        let before = FileTempoStore.load("t", d.path()).unwrap();
        assert_eq!(before.beats_per_bar, 3);
        let start_ms = before.ms_at(16);
        let new = TempoMap { beats_per_bar: 3, segments: map(90.0).segments };
        let got = FileTempoStore.replace("t", d.path(), &|_| Ok(new.clone())).unwrap();
        assert_eq!(got, new);
        let v = read(d.path());
        assert_eq!(v["future"], json!({"a": 1}));
        assert_eq!(v["speed"], json!(1.0));
        assert_eq!(v["position_ms"], json!(1234));
        assert_eq!(v["tempo"]["segments"][0]["bpm"], json!(90.0));
        let (st, _, _) = player_state::load(d.path());
        let Boundary::Tick { tick } = st.loops[0].start else { panic!("tick loop") };
        assert!((st.tempo.ms_at(tick) - start_ms).abs() < 60000.0 / 90.0 / 32.0, "the loop keeps its time");
    }

    #[test]
    fn newer_schema_is_refused_and_the_file_is_unchanged() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), json!({"schema_version": 6, "x": 1}));
        let bytes = std::fs::read(d.path().join("player.json")).unwrap();
        assert_eq!(FileTempoStore.replace("t", d.path(), &|_| Ok(map(100.0))).unwrap_err(), NEWER);
        assert_eq!(std::fs::read(d.path().join("player.json")).unwrap(), bytes);
    }

    #[test]
    fn a_missing_folder_is_not_created_and_f_errors_pass_through() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone");
        assert!(FileTempoStore.replace("t", &gone, &|_| Ok(map(100.0))).is_err());
        assert!(!gone.exists());
        write(d.path(), json!({"schema_version": 2}));
        assert_eq!(FileTempoStore.replace("t", d.path(), &|_| Err("no".into())).unwrap_err(), "no");
        assert!(FileTempoStore.load("t", &gone).unwrap().is_empty());
    }
}
