//! Tempo map and beat grid of the player. Pure logic, no Tauri, no IO.
//!
//! A beat is a quarter note; a tick is a 1/64 note (1/16 beat). The first segment's start is the
//! anchor (tick 0 = bar 1, beat 1); its tempo extends backwards (negative ticks). An empty map
//! means "no tempo" (timestamp mode).

use serde::{Deserialize, Serialize};

pub(crate) const TICKS_PER_BEAT: i64 = 16;
pub(crate) const DEFAULT_BEATS_PER_BAR: u32 = 4;
pub(crate) const MIN_BPM: f64 = 20.0;
pub(crate) const MAX_BPM: f64 = 300.0;
/// Detected changes move in 1/64 beat steps of the left section's tempo.
#[allow(dead_code)] // used by the IPC layer (later tasks) and the tests
pub(crate) const FINE_STEPS_PER_BEAT: i64 = 64;
/// Largest total offset of a detected change from `detected_ms`, in fine steps (4 beats).
#[allow(dead_code)] // used by the IPC layer (later tasks) and the tests
pub(crate) const MAX_OFFSET_STEPS: i64 = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: f64,
    pub bpm: f64,
    /// Position given by beat detection (on the left section's fine grid). Present = a detected
    /// change; absent = a typed change or the first segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TempoMap {
    pub beats_per_bar: u32,
    pub segments: Vec<Segment>,
}

/// A loop boundary: a grid tick (BPM mode) or an absolute time (timestamp mode).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Boundary {
    Tick { tick: i64 },
    Ms { ms: f64 },
}

impl Default for TempoMap {
    fn default() -> Self {
        TempoMap { beats_per_bar: DEFAULT_BEATS_PER_BAR, segments: Vec::new() }
    }
}

/// Valid typed values: 0, or 20.0..=300.0 with at most one decimal. Returns the value.
pub(crate) fn validate_bpm(bpm: f64) -> Result<f64, String> {
    if bpm == 0.0 {
        return Ok(0.0);
    }
    let one_decimal = bpm.is_finite() && ((bpm * 10.0).round() - bpm * 10.0).abs() < 1e-6;
    if !one_decimal || !(MIN_BPM..=MAX_BPM).contains(&bpm) {
        return Err(format!("BPM must be 0 or between {MIN_BPM} and {MAX_BPM} with one decimal"));
    }
    Ok((bpm * 10.0).round() / 10.0)
}

pub(crate) fn tick_ms(bpm: f64) -> f64 {
    60000.0 / (bpm * TICKS_PER_BEAT as f64)
}

#[allow(dead_code)] // used by the IPC layer (later tasks) and the tests
pub(crate) fn fine_ms(bpm: f64) -> f64 {
    60000.0 / (bpm * FINE_STEPS_PER_BEAT as f64)
}

/// The stretched last beat of a section before a detected change.
struct Stretch {
    /// First tick of the beat and its time on the section's own grid.
    a: i64,
    a_ms: f64,
    /// The change: its base tick and start time.
    end: i64,
    end_ms: f64,
}

impl TempoMap {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// More than one segment (the "V" marker).
    pub fn is_multi(&self) -> bool {
        self.segments.len() > 1
    }

    /// Tick base `T_i` of every segment.
    pub fn bases(&self) -> Vec<i64> {
        let mut out = Vec::with_capacity(self.segments.len());
        let mut t = 0i64;
        for (i, s) in self.segments.iter().enumerate() {
            if i > 0 {
                let p = &self.segments[i - 1];
                let ticks = (s.start_ms - p.start_ms) / tick_ms(p.bpm);
                if s.detected_ms.is_some() {
                    let beat = TICKS_PER_BEAT as f64;
                    t = TICKS_PER_BEAT * ((t as f64 + ticks) / beat).round() as i64;
                } else {
                    t += ticks.round() as i64;
                }
            }
            out.push(t);
        }
        out
    }

    /// Segment in effect at `ms` (the first one before the anchor); None when the map is empty.
    pub fn segment_at(&self, ms: f64) -> Option<usize> {
        if self.segments.is_empty() {
            return None;
        }
        Some(self.segments.iter().rposition(|s| s.start_ms <= ms).unwrap_or(0))
    }

    pub fn bpm_at(&self, ms: f64) -> f64 {
        self.segment_at(ms).map_or(0.0, |i| self.segments[i].bpm)
    }

    fn segment_for_tick(&self, bases: &[i64], tick: i64) -> usize {
        bases.iter().rposition(|&b| b <= tick).unwrap_or(0)
    }

    /// The stretched last beat of segment `i`, when the next segment is a detected change.
    fn stretch(&self, bases: &[i64], i: usize) -> Option<Stretch> {
        let next = self.segments.get(i + 1)?;
        next.detected_ms?;
        let end = bases[i + 1];
        let a = (end - TICKS_PER_BEAT).max(bases[i]);
        if a >= end {
            return None;
        }
        let s = &self.segments[i];
        Some(Stretch { a, a_ms: s.start_ms + (a - bases[i]) as f64 * tick_ms(s.bpm), end, end_ms: next.start_ms })
    }

    /// Time of a tick. An empty map gives 0.
    pub fn ms_at(&self, tick: i64) -> f64 {
        if self.segments.is_empty() {
            return 0.0;
        }
        let bases = self.bases();
        let i = self.segment_for_tick(&bases, tick);
        let s = &self.segments[i];
        if let Some(st) = self.stretch(&bases, i).filter(|st| tick >= st.a) {
            return st.a_ms + (tick - st.a) as f64 * (st.end_ms - st.a_ms) / (st.end - st.a) as f64;
        }
        s.start_ms + (tick - bases[i]) as f64 * tick_ms(s.bpm)
    }

    /// Nearest tick (ties away from zero). An empty map gives 0.
    pub fn tick_near(&self, ms: f64) -> i64 {
        let Some(i) = self.segment_at(ms) else { return 0 };
        let s = &self.segments[i];
        let bases = self.bases();
        if let Some(st) = self.stretch(&bases, i).filter(|st| ms >= st.a_ms) {
            let t = (ms - st.a_ms) / (st.end_ms - st.a_ms) * (st.end - st.a) as f64;
            return st.a + (t.round() as i64).min(st.end - st.a);
        }
        bases[i] + ((ms - s.start_ms) / tick_ms(s.bpm)).round() as i64
    }

    /// Beats with `a <= ms < b` as (ms, accent); accent = first beat of a bar.
    pub fn beats_in(&self, a: f64, b: f64) -> Vec<(f64, bool)> {
        let mut out = Vec::new();
        if self.segments.is_empty() || a.partial_cmp(&b) != Some(std::cmp::Ordering::Less) {
            return out;
        }
        let tpb = TICKS_PER_BEAT;
        let bar = tpb * self.beats_per_bar.max(1) as i64;
        let mut tick = (self.tick_near(a).saturating_sub(tpb)).div_euclid(tpb).saturating_mul(tpb);
        loop {
            let ms = self.ms_at(tick);
            if ms >= b {
                break;
            }
            if ms >= a {
                out.push((ms, tick.rem_euclid(bar) == 0));
            }
            tick = tick.saturating_add(tpb);
        }
        out
    }

    /// Type a BPM at a position (see plan 2.3). `bpm` must be valid (`validate_bpm`).
    pub fn set_bpm_at(&mut self, position_ms: f64, bpm: f64) -> Result<(), String> {
        let bpm = validate_bpm(bpm)?;
        if self.segments.is_empty() {
            if bpm > 0.0 {
                self.segments.push(Segment { start_ms: position_ms, bpm, detected_ms: None });
            }
            return Ok(());
        }
        if bpm == 0.0 {
            let i = self.segment_at(position_ms).unwrap_or(0);
            self.segments.remove(i);
            if let Some(first) = self.segments.first_mut() {
                first.detected_ms = None;
            }
        } else {
            let tick = self.tick_near(position_ms);
            let bases = self.bases();
            if tick <= 0 {
                self.segments[0].bpm = bpm;
            } else if let Some(i) = bases.iter().position(|&b| b == tick) {
                self.segments[i].bpm = bpm;
            } else {
                let start_ms = self.ms_at(tick);
                let at = self.segments.iter().position(|s| s.start_ms > start_ms).unwrap_or(self.segments.len());
                self.segments.insert(at, Segment { start_ms, bpm, detected_ms: None });
            }
        }
        self.segments.dedup_by(|b, a| a.bpm == b.bpm);
        Ok(())
    }

    /// Move detected change `index` to `detected_ms + steps * fine_ms(left bpm)`, clamped to one
    /// beat from its neighbours. Later changes keep their `start_ms`. Returns the new map and the
    /// applied steps.
    pub fn set_change_offset(&self, index: usize, steps: i64) -> Result<(TempoMap, i64), String> {
        if index == 0 || index >= self.segments.len() {
            return Err("The first tempo section can't be moved".to_string());
        }
        let seg = &self.segments[index];
        let Some(detected) = seg.detected_ms else {
            return Err("Only detected tempo changes can be moved; type the BPM again in the Player to change this one"
                .to_string());
        };
        if steps.abs() > MAX_OFFSET_STEPS {
            return Err(format!("A tempo change can move at most {MAX_OFFSET_STEPS} steps from its detected position"));
        }
        let prev = &self.segments[index - 1];
        let fine = fine_ms(prev.bpm);
        let lo = prev.start_ms + 60000.0 / prev.bpm;
        let hi = self.segments.get(index + 1).map(|n| n.start_ms - 60000.0 / seg.bpm);
        let at = |n: i64| detected + n as f64 * fine;
        let mut min = (-MAX_OFFSET_STEPS).max(((lo - detected) / fine).ceil().max(-1e9) as i64);
        while at(min) < lo {
            min += 1;
        }
        let mut max = MAX_OFFSET_STEPS;
        if let Some(hi) = hi {
            max = max.min(((hi - detected) / fine).floor().min(1e9) as i64);
            while at(max) > hi {
                max -= 1;
            }
        }
        if min > max {
            return Err("There is no room to move this tempo change".to_string());
        }
        let applied = steps.clamp(min, max);
        let mut map = self.clone();
        map.segments[index].start_ms = at(applied);
        Ok((map, applied))
    }
}

/// Re-express a loop boundary for a new map, keeping its time (within half a tick).
pub(crate) fn resnap(boundary: Boundary, old: &TempoMap, new: &TempoMap) -> Boundary {
    let ms = match boundary {
        Boundary::Tick { tick } => {
            if old.is_empty() {
                return boundary; // cannot happen in a consistent state
            }
            old.ms_at(tick)
        }
        Boundary::Ms { ms } => ms,
    };
    if new.is_empty() {
        Boundary::Ms { ms }
    } else {
        Boundary::Tick { tick: new.tick_near(ms) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(segs: &[(f64, f64)]) -> TempoMap {
        TempoMap {
            beats_per_bar: 4,
            segments: segs.iter().map(|&(start_ms, bpm)| Segment { start_ms, bpm, detected_ms: None }).collect(),
        }
    }
    fn with_tempo() -> TempoMap {
        map(&[(500.0, 120.0), (10500.0, 90.0)])
    }

    #[test]
    fn one_segment() {
        let m = map(&[(500.0, 120.0)]);
        assert_eq!(m.ms_at(64), 2500.0);
        assert_eq!(m.tick_near(2510.0), 64);
        assert_eq!(m.tick_near(2515.7), 65);
        assert_eq!(m.ms_at(-16), 0.0);
        assert!(!m.is_multi());
    }

    #[test]
    fn with_tempo_map() {
        let m = with_tempo();
        assert_eq!(m.bases(), vec![0, 320]);
        assert!((m.ms_at(384) - 13166.667).abs() < 1e-3);
        assert!((m.ms_at(384) - (10500.0 + 64.0 * 60000.0 / 1440.0)).abs() < 1e-6);
        assert!((m.ms_at(512) - 18500.0).abs() < 1e-6);
        assert_eq!(m.bpm_at(10499.0), 120.0);
        assert_eq!(m.bpm_at(10500.0), 90.0);
        assert!(m.is_multi());
    }

    #[test]
    fn beats() {
        let m = map(&[(500.0, 120.0)]);
        assert_eq!(m.beats_in(250.0, 2000.0), vec![(500.0, true), (1000.0, false), (1500.0, false)]);
        assert!(TempoMap::default().beats_in(0.0, 5000.0).is_empty());
    }

    #[test]
    fn set_bpm_empty_and_snap() {
        let mut m = TempoMap::default();
        m.set_bpm_at(1234.0, 120.0).unwrap();
        assert_eq!(m.segments, vec![Segment { start_ms: 1234.0, bpm: 120.0, detected_ms: None }]);

        let mut m = map(&[(500.0, 120.0)]);
        m.set_bpm_at(10510.0, 90.0).unwrap();
        assert_eq!(m, with_tempo());
    }

    #[test]
    fn set_bpm_insert_between() {
        let mut m = with_tempo();
        m.set_bpm_at(5000.0, 100.0).unwrap();
        assert_eq!(m.segments.len(), 3);
        assert_eq!(m.segments[1], Segment { start_ms: 5000.0, bpm: 100.0, detected_ms: None });
        assert_eq!(m.segments[2].start_ms, 10500.0);
        assert_eq!(m.bpm_at(4999.0), 120.0);
        assert_eq!(m.bpm_at(9000.0), 100.0);
        assert_eq!(m.bpm_at(11000.0), 90.0);
    }

    #[test]
    fn set_bpm_replace_before_anchor_and_remove() {
        let mut m = with_tempo();
        m.set_bpm_at(100.0, 110.0).unwrap();
        assert_eq!(m.segments[0].bpm, 110.0);
        let mut m = with_tempo();
        m.set_bpm_at(12000.0, 0.0).unwrap();
        assert_eq!(m.segments, vec![Segment { start_ms: 500.0, bpm: 120.0, detected_ms: None }]);
        m.set_bpm_at(12000.0, 0.0).unwrap();
        assert!(m.is_empty());
        m.set_bpm_at(12000.0, 0.0).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn equal_neighbours_merge() {
        let mut m = with_tempo();
        m.set_bpm_at(10500.0, 120.0).unwrap();
        assert_eq!(m.segments.len(), 1);
        let mut m = with_tempo();
        m.set_bpm_at(5000.0, 90.0).unwrap();
        assert_eq!(m.segments.len(), 2);
        assert_eq!(m.segments[1].start_ms, 5000.0);
    }

    #[test]
    fn invalid_bpm_refused() {
        let mut m = with_tempo();
        assert!(m.set_bpm_at(1000.0, 19.9).is_err());
        assert!(m.set_bpm_at(1000.0, 300.1).is_err());
        assert!(m.set_bpm_at(1000.0, 100.25).is_err());
        assert!(m.set_bpm_at(1000.0, f64::NAN).is_err());
        assert_eq!(m, with_tempo());
        assert_eq!(validate_bpm(20.0), Ok(20.0));
        assert_eq!(validate_bpm(300.0), Ok(300.0));
        assert_eq!(validate_bpm(0.0), Ok(0.0));
        assert_eq!(validate_bpm(120.5), Ok(120.5));
    }

    #[test]
    fn resnap_keeps_time() {
        let old = with_tempo();
        let mut new = old.clone();
        new.set_bpm_at(5000.0, 100.0).unwrap();
        for tick in [64, 192, 384, 512] {
            let ms = old.ms_at(tick);
            let Boundary::Tick { tick: t2 } = resnap(Boundary::Tick { tick }, &old, &new) else {
                panic!("expected tick")
            };
            let half = 60000.0 / (new.bpm_at(ms) * 16.0) / 2.0;
            assert!((new.ms_at(t2) - ms).abs() <= half + 1e-6);
        }
    }

    #[test]
    fn resnap_converts_kinds() {
        let m = with_tempo();
        let empty = TempoMap::default();
        match resnap(Boundary::Tick { tick: 64 }, &m, &empty) {
            Boundary::Ms { ms } => assert_eq!(ms, 2500.0),
            b => panic!("{b:?}"),
        }
        assert_eq!(resnap(Boundary::Ms { ms: 2500.0 }, &empty, &m), Boundary::Tick { tick: 64 });
    }

    #[test]
    fn boundary_serde() {
        assert_eq!(serde_json::to_string(&Boundary::Tick { tick: 64 }).unwrap(), r#"{"tick":64}"#);
        assert_eq!(serde_json::from_str::<Boundary>(r#"{"ms":12.5}"#).unwrap(), Boundary::Ms { ms: 12.5 });
    }

    /// 120 BPM first section (500 ms anchor), a detected change at `detected + steps` fine steps,
    /// then a typed 90 BPM change far away and another detected one.
    fn detected(detected_ms: f64, start_ms: f64) -> TempoMap {
        let mut m = map(&[(500.0, 120.0), (0.0, 90.0), (30000.0, 100.0)]);
        m.segments[1] = Segment { start_ms, bpm: 90.0, detected_ms: Some(detected_ms) };
        m.segments[2].detected_ms = Some(30000.0);
        m
    }

    #[test]
    fn detected_change_offsets_and_stretch() {
        let fine = fine_ms(120.0);
        assert_eq!(fine, 7.8125);
        for steps in [0i64, 1, 3, -5, 256] {
            let base = detected(10300.0, 10300.0);
            let (m, applied) = base.set_change_offset(1, steps).unwrap();
            assert_eq!(applied, steps);
            let start = 10300.0 + steps as f64 * fine;
            assert_eq!(m.segments[1].start_ms, start);
            assert_eq!(m.segments[1].detected_ms, Some(10300.0));
            // later change untouched
            assert_eq!(m.segments[2].start_ms, 30000.0);
            assert_eq!(m.segments[2].detected_ms, Some(30000.0));
            let bases = m.bases();
            assert_eq!(bases[1] % 16, 0);
            assert!(bases[2] - bases[1] >= 16);
            // new section's beats start exactly at start_ms
            let beats = m.beats_in(start, start + 1.0);
            assert_eq!(beats, vec![(start, bases[1] % 64 == 0)]);
            // monotonic and inverse around the change, stretched beat within 1/2..3/2 beat
            let mut prev = f64::NEG_INFINITY;
            for t in bases[1] - 40..bases[1] + 40 {
                let ms = m.ms_at(t);
                assert!(ms > prev, "tick {t}");
                prev = ms;
                assert_eq!(m.tick_near(ms), t, "tick {t}");
            }
            assert_eq!(m.ms_at(bases[1]), start);
            let len = start - m.ms_at(bases[1] - 16);
            assert!((250.0 - 1e-9..=750.0 + 1e-9).contains(&len), "len {len}");
        }
    }

    #[test]
    fn detected_change_refusals_and_clamps() {
        let m = detected(10300.0, 10300.0);
        assert!(m.set_change_offset(1, 257).is_err());
        assert!(m.set_change_offset(1, -257).is_err());
        assert!(m.set_change_offset(0, 0).is_err());
        let mut typed = m.clone();
        typed.segments[1].detected_ms = None;
        assert_eq!(
            typed.set_change_offset(1, 1).map(|_| ()).unwrap_err(),
            "Only detected tempo changes can be moved; type the BPM again in the Player to change this one"
        );
        // upper limit: the next change (100 BPM... the moved one is 90 BPM) keeps one beat of room
        let mut c = detected(29000.0, 29000.0);
        c.segments[2].start_ms = 29900.0;
        c.segments[2].detected_ms = Some(29900.0);
        let (m2, applied) = c.set_change_offset(1, 100).unwrap();
        let limit = 29900.0 - 60000.0 / 90.0;
        assert!(m2.segments[1].start_ms <= limit && limit - m2.segments[1].start_ms < fine_ms(120.0));
        assert_eq!(m2.segments[1].start_ms, 29000.0 + applied as f64 * fine_ms(120.0));
        assert!(applied < 100);
        // lower limit: one beat of 120 BPM after the anchor
        let (m3, applied) = detected(1100.0, 1100.0).set_change_offset(1, -256).unwrap();
        assert!(m3.segments[1].start_ms >= 1000.0 && m3.segments[1].start_ms - 1000.0 < fine_ms(120.0));
        assert!(applied > -256);
    }

    #[test]
    fn moving_one_change_keeps_the_next() {
        let m = detected(10300.0, 10300.0);
        let (m2, _) = m.set_change_offset(1, 40).unwrap();
        assert_eq!(m2.segments[2], m.segments[2]);
        assert!(m2.set_change_offset(1, 0).unwrap().0 == m);
    }

    #[test]
    fn typed_maps_have_no_stretch() {
        let m = with_tempo();
        assert!(m.stretch(&m.bases(), 0).is_none());
    }

    #[test]
    fn anchor_rules_through_set_bpm_at() {
        // replace at a detected change's start: bpm changes, anchor and start stay
        let mut m = detected(10300.0, 10316.0);
        let b = m.bases()[1];
        let at = m.ms_at(b);
        m.set_bpm_at(at, 80.0).unwrap();
        assert_eq!(m.segments[1].bpm, 80.0);
        assert_eq!(m.segments[1].start_ms, 10316.0);
        assert_eq!(m.segments[1].detected_ms, Some(10300.0));
        // typed insert elsewhere: no anchor, detected keep theirs
        let mut m = detected(10300.0, 10300.0);
        m.set_bpm_at(5000.0, 110.0).unwrap();
        assert_eq!(m.segments.len(), 4);
        assert_eq!(m.segments[1].detected_ms, None);
        assert_eq!(m.segments[2].detected_ms, Some(10300.0));
        assert_eq!(m.segments[3].detected_ms, Some(30000.0));
        // removal of a detected change takes its anchor
        let mut m = detected(10300.0, 10300.0);
        m.set_bpm_at(15000.0, 0.0).unwrap();
        assert_eq!(m.segments.len(), 2);
        assert_eq!(m.segments[1].start_ms, 30000.0);
        // removing the first segment: the new first drops its anchor
        let mut m = detected(10300.0, 10300.0);
        m.set_bpm_at(100.0, 0.0).unwrap();
        assert_eq!(m.segments[0].start_ms, 10300.0);
        assert_eq!(m.segments[0].detected_ms, None);
        // merge of equal neighbours: the later one (and its anchor) goes
        let mut m = detected(10300.0, 10300.0);
        m.set_bpm_at(10300.0, 120.0).unwrap();
        assert_eq!(m.segments.len(), 2);
        assert_eq!(m.segments[1].start_ms, 30000.0);
        // serde: absent anchor is not written, old json reads
        assert!(!serde_json::to_string(&with_tempo().segments[0]).unwrap().contains("detected"));
        let s: Segment = serde_json::from_str(r#"{"start_ms":1.0,"bpm":100.0}"#).unwrap();
        assert_eq!(s.detected_ms, None);
    }
}
