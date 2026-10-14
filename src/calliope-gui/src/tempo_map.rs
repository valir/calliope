//! Tempo map and beat grid of the player. Pure logic, no Tauri, no IO.
//!
//! A beat is a quarter note; a tick is a 1/64 note (1/16 beat). The first segment's start is the
//! anchor (tick 0 = bar 1, beat 1); its tempo extends backwards (negative ticks). An empty map
//! means "no tempo" (timestamp mode).
//!
//! Anchor rule for typed BPMs: a BPM typed at a change's start, or inside the stretched last beat
//! before a following detected change, replaces that change's BPM (its position and `detected_ms`
//! are kept). `bases()` additionally keeps tick bases strictly increasing.

use serde::{Deserialize, Serialize};

pub(crate) const TICKS_PER_BEAT: i64 = 16;
pub(crate) const DEFAULT_BEATS_PER_BAR: u32 = 4;
pub(crate) const MIN_BPM: f64 = 20.0;
pub(crate) const MAX_BPM: f64 = 300.0;
/// Detected changes move in 1/64 beat steps of the left section's tempo.
pub(crate) const FINE_STEPS_PER_BEAT: i64 = 64;
/// Largest total offset of a detected change from `detected_ms`, in fine steps (4 beats).
pub(crate) const MAX_OFFSET_STEPS: i64 = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: f64,
    pub bpm: f64,
    /// Position given by beat detection (on the left section's fine grid). Present = a detected
    /// change; absent = a typed change or the first segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_ms: Option<f64>,
    /// A placed change (or a typed one that was moved): its start is on a beat, like a detected
    /// change's. Never set on the first segment.
    #[serde(default, skip_serializing_if = "is_false")]
    pub on_beat: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Segment {
    /// Detected and placed changes start on a beat (beat-rounded base, stretched last beat before).
    pub fn starts_on_beat(&self) -> bool {
        self.detected_ms.is_some() || self.on_beat
    }
}

/// Factor of `TempoMap::scale_section`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Double,
    Halve,
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
                if s.starts_on_beat() {
                    let beat = TICKS_PER_BEAT as f64;
                    let aligned = TICKS_PER_BEAT * ((t as f64 + ticks) / beat).round() as i64;
                    // safety net: bases stay strictly increasing (next beat boundary at least)
                    t = aligned.max((t.div_euclid(TICKS_PER_BEAT) + 1) * TICKS_PER_BEAT);
                } else {
                    t = (t + ticks.round() as i64).max(t + 1);
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
        if !next.starts_on_beat() {
            return None;
        }
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
                self.segments.push(Segment { start_ms: position_ms, bpm, detected_ms: None, on_beat: false });
            }
            return Ok(());
        }
        if bpm == 0.0 {
            let i = self.segment_at(position_ms).unwrap_or(0);
            self.segments.remove(i);
            if let Some(first) = self.segments.first_mut() {
                first.detected_ms = None;
                first.on_beat = false;
            }
        } else {
            let tick = self.tick_near(position_ms);
            let bases = self.bases();
            if tick <= 0 {
                self.segments[0].bpm = bpm;
            } else if let Some(i) = bases.iter().position(|&b| b == tick) {
                self.segments[i].bpm = bpm;
            } else if let Some(i) = bases
                .iter()
                .position(|&b| b > tick)
                .filter(|&i| self.segments[i].starts_on_beat() && tick > bases[i] - TICKS_PER_BEAT)
            {
                // inside the stretched last beat before a detected change: applied at that change
                self.segments[i].bpm = bpm;
            } else {
                let start_ms = self.ms_at(tick);
                let at = self.segments.iter().position(|s| s.start_ms > start_ms).unwrap_or(self.segments.len());
                self.segments.insert(at, Segment { start_ms, bpm, detected_ms: None, on_beat: false });
            }
        }
        self.segments.dedup_by(|b, a| a.bpm == b.bpm);
        Ok(())
    }

    /// The Editor's move: detected changes only, then `move_change` without a track end.
    pub fn set_change_offset(&self, index: usize, steps: i64) -> Result<(TempoMap, i64), String> {
        if index == 0 || index >= self.segments.len() {
            return Err("The first tempo section can't be moved".to_string());
        }
        if self.segments[index].detected_ms.is_none() {
            return Err("Only detected tempo changes can be moved; type the BPM again in the Player to change this one"
                .to_string());
        }
        self.move_change(index, steps, f64::INFINITY)
    }

    /// Move change `index` by `steps` fine steps of the left tempo (plan 2.4). A detected change
    /// counts from `detected_ms` (widened limit), any other from the left section's start. Clamped
    /// to one beat from its neighbours and from `end_ms`. Returns the new map and the applied steps.
    pub fn move_change(&self, index: usize, steps: i64, end_ms: f64) -> Result<(TempoMap, i64), String> {
        if index == 0 || index >= self.segments.len() {
            return Err("The first tempo section can't be moved".to_string());
        }
        let seg = &self.segments[index];
        let prev = &self.segments[index - 1];
        let fine = fine_ms(prev.bpm);
        let reference = seg.detected_ms.unwrap_or(prev.start_ms);
        let mut lim = i64::MAX;
        if seg.detected_ms.is_some() {
            let cur = ((seg.start_ms - reference) / fine).round() as i64;
            lim = MAX_OFFSET_STEPS.max(cur.unsigned_abs() as i64);
            if steps.unsigned_abs() > lim as u64 {
                return Err(format!(
                    "A tempo change can move at most {MAX_OFFSET_STEPS} steps from its detected position"
                ));
            }
        }
        let lo = prev.start_ms + 60000.0 / prev.bpm;
        let beat = 60000.0 / seg.bpm;
        let hi = self.segments.get(index + 1).map_or(end_ms, |n| n.start_ms) - beat;
        let at = |n: i64| reference + n as f64 * fine;
        let mut min = (-lim).max(((lo - reference) / fine).ceil().max(-1e9) as i64);
        while at(min) < lo {
            min += 1;
        }
        let mut max = lim;
        if hi.is_finite() {
            max = max.min(((hi - reference) / fine).floor().min(1e9) as i64);
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
        if !seg.starts_on_beat() {
            map.segments[index].on_beat = true;
        }
        Ok((map, applied))
    }

    /// Set the BPM of section `index` (20..=300, one decimal). Positions and flags are kept and
    /// equal neighbours are not merged.
    pub fn set_section_bpm(&self, index: usize, bpm: f64) -> Result<TempoMap, String> {
        let bpm = section_bpm(bpm)?;
        let seg = self.segments.get(index).ok_or_else(|| format!("There is no tempo section {}", index + 1))?;
        if let Some(n) = self.segments.get(index + 1) {
            if n.start_ms - seg.start_ms < 60000.0 / bpm {
                return Err(format!("Section {} is too short for {bpm} BPM (it needs at least one beat)", index + 1));
            }
        }
        let mut map = self.clone();
        map.segments[index].bpm = bpm;
        Ok(map)
    }

    /// Double or halve the BPM of section `index` (rounded to one decimal).
    pub fn scale_section(&self, index: usize, factor: Scale) -> Result<TempoMap, String> {
        let seg = self.segments.get(index).ok_or_else(|| format!("There is no tempo section {}", index + 1))?;
        let f = if factor == Scale::Double { 2.0 } else { 0.5 };
        let bpm = (seg.bpm * f * 10.0).round() / 10.0;
        if bpm > MAX_BPM {
            return Err(format!("{bpm} BPM is over {MAX_BPM}"));
        }
        if bpm < MIN_BPM {
            return Err(format!("{bpm} BPM is under {MIN_BPM}"));
        }
        self.set_section_bpm(index, bpm)
    }

    /// Add a placed change near `position_ms` (on the split section's fine grid). Returns the new
    /// map and the index of the new section.
    pub fn add_change(&self, position_ms: f64, bpm: f64, end_ms: f64) -> Result<(TempoMap, usize), String> {
        if self.segments.is_empty() {
            return Err("Enter a BPM first".to_string());
        }
        let bpm = section_bpm(bpm)?;
        if !position_ms.is_finite() {
            return Err("Add tempo changes inside the track".to_string());
        }
        if position_ms < self.segments[0].start_ms {
            return Err("Add tempo changes after the start of the first tempo".to_string());
        }
        if position_ms >= end_ms {
            return Err("Add tempo changes inside the track".to_string());
        }
        let k = self.segment_at(position_ms).unwrap_or(0);
        let left = &self.segments[k];
        let fine = fine_ms(left.bpm);
        let start = left.start_ms + ((position_ms - left.start_ms) / fine).round() * fine;
        let next_start = self.segments.get(k + 1).map_or(end_ms, |n| n.start_ms);
        if start - left.start_ms < 60000.0 / left.bpm || next_start - start < 60000.0 / bpm {
            return Err("A tempo change needs at least one beat on each side".to_string());
        }
        let mut map = self.clone();
        map.segments.insert(k + 1, Segment { start_ms: start, bpm, detected_ms: None, on_beat: true });
        Ok((map, k + 1))
    }

    /// Remove change `index`: its section joins the previous one.
    pub fn remove_change(&self, index: usize) -> Result<TempoMap, String> {
        if index == 0 || index >= self.segments.len() {
            return Err("The first section has no tempo change to remove".to_string());
        }
        let mut map = self.clone();
        map.segments.remove(index);
        Ok(map)
    }
}

/// A section BPM: 20..=300 with one decimal (0 is refused).
fn section_bpm(bpm: f64) -> Result<f64, String> {
    if bpm == 0.0 {
        return Err("Enter a BPM between 20 and 300".to_string());
    }
    validate_bpm(bpm)
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
            segments: segs.iter().map(|&(start_ms, bpm)| Segment { start_ms, bpm, detected_ms: None, on_beat: false }).collect(),
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
        assert_eq!(m.segments, vec![Segment { start_ms: 1234.0, bpm: 120.0, detected_ms: None, on_beat: false }]);

        let mut m = map(&[(500.0, 120.0)]);
        m.set_bpm_at(10510.0, 90.0).unwrap();
        assert_eq!(m, with_tempo());
    }

    #[test]
    fn set_bpm_insert_between() {
        let mut m = with_tempo();
        m.set_bpm_at(5000.0, 100.0).unwrap();
        assert_eq!(m.segments.len(), 3);
        assert_eq!(m.segments[1], Segment { start_ms: 5000.0, bpm: 100.0, detected_ms: None, on_beat: false });
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
        assert_eq!(m.segments, vec![Segment { start_ms: 500.0, bpm: 120.0, detected_ms: None, on_beat: false }]);
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
        m.segments[1] = Segment { start_ms, bpm: 90.0, detected_ms: Some(detected_ms), on_beat: false };
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

    fn assert_monotonic_around(m: &TempoMap, i: usize) {
        let bases = m.bases();
        let mut prev = f64::NEG_INFINITY;
        for t in bases[i] - 40..bases[i] + 40 {
            let ms = m.ms_at(t);
            assert!(ms > prev, "tick {t}");
            prev = ms;
            assert_eq!(m.tick_near(ms), t, "tick {t}");
        }
    }

    #[test]
    fn add_change_lands_on_the_grid_and_on_a_beat() {
        let m = map(&[(500.0, 120.0)]);
        let (n, idx) = m.add_change(10010.0, 93.0, 200000.0).unwrap();
        assert_eq!(idx, 1);
        let fine = fine_ms(120.0);
        let steps = ((10010.0 - 500.0) / fine).round();
        let s = &n.segments[1];
        assert!((s.start_ms - (500.0 + steps * fine)).abs() < 1e-9);
        assert!((s.start_ms - 10010.0).abs() <= fine / 2.0 + 1e-9);
        assert!(s.on_beat && s.detected_ms.is_none() && s.starts_on_beat());
        assert_eq!(s.bpm, 93.0);
        assert_eq!(n.beats_in(s.start_ms, s.start_ms + 1.0).len(), 1);
        assert_eq!(n.beats_in(s.start_ms, s.start_ms + 1.0)[0].0, s.start_ms);
        assert_eq!(n.bases()[1] % 16, 0);
        assert_monotonic_around(&n, 1);
        assert_eq!(m.segments.len(), 1);
        // splitting the middle section keeps the later change
        let (n2, idx) = with_tempo().add_change(5000.0, 100.0, 60000.0).unwrap();
        assert_eq!((idx, n2.segments.len(), n2.segments[2].start_ms), (1, 3, 10500.0));
        assert_monotonic_around(&n2, 1);
        assert_monotonic_around(&n2, 2);
    }

    #[test]
    fn add_change_refusals() {
        let m = map(&[(500.0, 120.0)]);
        let err = |m: &TempoMap, pos: f64, bpm: f64, end: f64| m.add_change(pos, bpm, end).map(|_| ()).unwrap_err();
        assert_eq!(err(&TempoMap::default(), 1000.0, 100.0, 1e6), "Enter a BPM first");
        assert_eq!(err(&m, 100.0, 100.0, 1e6), "Add tempo changes after the start of the first tempo");
        assert_eq!(err(&m, 5000.0, 100.0, 5000.0), "Add tempo changes inside the track");
        assert_eq!(err(&m, 5000.0, 0.0, 1e6), "Enter a BPM between 20 and 300");
        assert!(m.add_change(5000.0, 301.0, 1e6).is_err());
        let side = "A tempo change needs at least one beat on each side";
        assert_eq!(err(&m, 700.0, 100.0, 1e6), side); // within a beat of the anchor
        assert_eq!(err(&with_tempo(), 10200.0, 100.0, 1e6), side); // within a beat of the next change
        assert_eq!(err(&with_tempo(), 10600.0, 100.0, 10900.0), side); // too close to the end
        assert!(m.add_change(1000.0, 100.0, 1e6).is_ok()); // exactly one beat after the anchor
    }

    #[test]
    fn move_change_placed_typed_and_detected() {
        // placed: counted from the left start, no 256 limit
        let (n, _) = with_tempo().add_change(5000.0, 100.0, 1e6).unwrap();
        let (m, applied) = n.move_change(1, 1000, 1e6).unwrap();
        assert_eq!(applied, 1000);
        assert_eq!(m.segments[1].start_ms, 500.0 + 1000.0 * fine_ms(120.0));
        assert!(m.segments[1].on_beat);
        // typed: first move turns it into a placed change
        assert!(!with_tempo().segments[1].on_beat);
        let (m, applied) = with_tempo().move_change(1, 1300, 1e6).unwrap();
        assert_eq!(applied, 1300);
        assert_eq!(m.segments[1].start_ms, 500.0 + 1300.0 * fine_ms(120.0));
        assert!(m.segments[1].on_beat && m.segments[1].detected_ms.is_none());
        // detected keeps +-256 and its detected_ms
        let d = detected(10300.0, 10300.0);
        assert!(d.move_change(1, 257, 1e6).is_err());
        let (m, _) = d.move_change(1, 256, 1e6).unwrap();
        assert_eq!(m.segments[1].detected_ms, Some(10300.0));
        assert!(!m.segments[1].on_beat);
        // the Editor refuses typed and placed ones
        let msg = "Only detected tempo changes can be moved; type the BPM again in the Player to change this one";
        assert_eq!(with_tempo().set_change_offset(1, 1).map(|_| ()).unwrap_err(), msg);
        assert_eq!(n.set_change_offset(1, 1).map(|_| ()).unwrap_err(), msg);
    }

    #[test]
    fn move_change_widened_limit_and_clamps() {
        // left 60 BPM, change at +200 steps; doubling the left BPM makes it +400
        let mut d = map(&[(500.0, 60.0), (0.0, 90.0), (60000.0, 100.0)]);
        d.segments[1] = Segment { start_ms: 10500.0 + 200.0 * fine_ms(60.0), bpm: 90.0, detected_ms: Some(10500.0), on_beat: false };
        let d = d.set_section_bpm(0, 120.0).unwrap();
        assert_eq!(d.move_change(1, 400, 1e6).unwrap().1, 400);
        assert_eq!(d.move_change(1, 399, 1e6).unwrap().1, 399);
        assert!(d.move_change(1, 401, 1e6).is_err());
        assert!(d.move_change(1, -401, 1e6).is_err());
        // clamps: left neighbour, right neighbour, end of the track
        let m = with_tempo();
        let low = m.move_change(1, -100000, 1e6).unwrap().0.segments[1].start_ms;
        assert!(low >= 1000.0 && low - 1000.0 < fine_ms(120.0));
        let (n, _) = m.add_change(20000.0, 100.0, 1e6).unwrap();
        let high = n.move_change(1, 100000, 1e6).unwrap().0.segments[1].start_ms;
        let lim = 20000.0 - 60000.0 / 90.0;
        assert!(high <= lim && lim - high < fine_ms(120.0));
        let end = m.move_change(1, 100000, 12000.0).unwrap().0.segments[1].start_ms;
        let lim = 12000.0 - 60000.0 / 90.0;
        assert!(end <= lim && lim - end < fine_ms(120.0));
        assert!(m.move_change(1, 0, 1500.0).unwrap_err().contains("no room"));
        assert!(m.move_change(0, 0, 1e6).is_err() && m.move_change(5, 0, 1e6).is_err());
    }

    #[test]
    fn section_bpm_keeps_everything_else() {
        let d = detected(10300.0, 10316.0);
        let n = d.set_section_bpm(1, 91.5).unwrap();
        assert_eq!(n.segments[1], Segment { start_ms: 10316.0, bpm: 91.5, detected_ms: Some(10300.0), on_beat: false });
        assert_eq!((n.segments[0].clone(), n.segments[2].clone()), (d.segments[0].clone(), d.segments[2].clone()));
        // an equal neighbour is not merged
        assert_eq!(with_tempo().set_section_bpm(1, 120.0).unwrap().segments.len(), 2);
        // too short: a one-beat section set slower
        let p = map(&[(500.0, 120.0), (5000.0, 120.0), (5500.0, 120.0)]);
        let err = p.set_section_bpm(1, 100.0).unwrap_err();
        assert_eq!(err, "Section 2 is too short for 100 BPM (it needs at least one beat)");
        assert!(p.set_section_bpm(2, 100.0).is_ok());
        // validation
        assert_eq!(with_tempo().set_section_bpm(0, 0.0).unwrap_err(), "Enter a BPM between 20 and 300");
        assert!(with_tempo().set_section_bpm(0, 100.25).is_err());
        assert!(with_tempo().set_section_bpm(0, 301.0).is_err());
        assert!(with_tempo().set_section_bpm(7, 100.0).is_err());
    }

    #[test]
    fn scale_section_rounds_and_refuses() {
        let m = map(&[(500.0, 93.0)]);
        let up = m.scale_section(0, Scale::Double).unwrap();
        assert_eq!(up.segments[0].bpm, 186.0);
        assert_eq!(up.scale_section(0, Scale::Halve).unwrap().segments[0].bpm, 93.0);
        assert_eq!(map(&[(0.0, 93.5)]).scale_section(0, Scale::Halve).unwrap().segments[0].bpm, 46.8);
        assert_eq!(map(&[(0.0, 160.0)]).scale_section(0, Scale::Double).unwrap_err(), "320 BPM is over 300");
        assert_eq!(map(&[(0.0, 39.8)]).scale_section(0, Scale::Halve).unwrap_err(), "19.9 BPM is under 20");
    }

    #[test]
    fn remove_change_joins_sections() {
        let m = detected(10300.0, 10300.0);
        assert_eq!(m.remove_change(0).unwrap_err(), "The first section has no tempo change to remove");
        assert!(m.remove_change(3).is_err());
        let n = m.remove_change(1).unwrap();
        assert_eq!(n.segments.len(), 2);
        assert_eq!(n.segments[1], m.segments[2]);
        // the next detected change now stretches the previous (120 BPM) section's last beat
        assert_monotonic_around(&n, 1);
        let b = n.bases();
        assert_eq!(b[1] % 16, 0);
        assert_eq!(n.ms_at(b[1]), 30000.0);
        let last = m.remove_change(2).unwrap();
        assert_eq!(last.segments.len(), 2);
        assert_eq!(map(&[(500.0, 120.0), (10500.0, 90.0)]).remove_change(1).unwrap().segments.len(), 1);
    }

    #[test]
    fn on_beat_serde() {
        let mut s = Segment { start_ms: 1.0, bpm: 100.0, detected_ms: None, on_beat: false };
        assert!(!serde_json::to_string(&s).unwrap().contains("on_beat"));
        s.on_beat = true;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""on_beat":true"#));
        assert_eq!(serde_json::from_str::<Segment>(&json).unwrap(), s);
    }
}
