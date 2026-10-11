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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Segment {
    pub start_ms: f64,
    pub bpm: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TempoMap {
    pub beats_per_bar: u32,
    pub segments: Vec<Segment>,
}

/// A loop boundary: a grid tick (BPM mode) or an absolute time (timestamp mode).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Boundary {
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

fn tick_ms(bpm: f64) -> f64 {
    60000.0 / (bpm * TICKS_PER_BEAT as f64)
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
                t += ((s.start_ms - p.start_ms) / tick_ms(p.bpm)).round() as i64;
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

    /// Time of a tick. An empty map gives 0.
    pub fn ms_at(&self, tick: i64) -> f64 {
        if self.segments.is_empty() {
            return 0.0;
        }
        let bases = self.bases();
        let i = self.segment_for_tick(&bases, tick);
        let s = &self.segments[i];
        s.start_ms + (tick - bases[i]) as f64 * tick_ms(s.bpm)
    }

    /// Nearest tick (ties away from zero). An empty map gives 0.
    pub fn tick_near(&self, ms: f64) -> i64 {
        let Some(i) = self.segment_at(ms) else { return 0 };
        let s = &self.segments[i];
        self.bases()[i] + ((ms - s.start_ms) / tick_ms(s.bpm)).round() as i64
    }

    /// Beats with `a <= ms < b` as (ms, accent); accent = first beat of a bar.
    pub fn beats_in(&self, a: f64, b: f64) -> Vec<(f64, bool)> {
        let mut out = Vec::new();
        if self.segments.is_empty() || a.partial_cmp(&b) != Some(std::cmp::Ordering::Less) {
            return out;
        }
        let tpb = TICKS_PER_BEAT;
        let bar = tpb * self.beats_per_bar.max(1) as i64;
        let mut tick = (self.tick_near(a) - tpb).div_euclid(tpb) * tpb;
        loop {
            let ms = self.ms_at(tick);
            if ms >= b {
                break;
            }
            if ms >= a {
                out.push((ms, tick.rem_euclid(bar) == 0));
            }
            tick += tpb;
        }
        out
    }

    /// Type a BPM at a position (see plan 2.3). `bpm` must be valid (`validate_bpm`).
    pub fn set_bpm_at(&mut self, position_ms: f64, bpm: f64) -> Result<(), String> {
        let bpm = validate_bpm(bpm)?;
        if self.segments.is_empty() {
            if bpm > 0.0 {
                self.segments.push(Segment { start_ms: position_ms, bpm });
            }
            return Ok(());
        }
        if bpm == 0.0 {
            let i = self.segment_at(position_ms).unwrap_or(0);
            self.segments.remove(i);
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
                self.segments.insert(at, Segment { start_ms, bpm });
            }
        }
        self.segments.dedup_by(|b, a| a.bpm == b.bpm);
        Ok(())
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
            segments: segs.iter().map(|&(start_ms, bpm)| Segment { start_ms, bpm }).collect(),
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
        assert_eq!(m.segments, vec![Segment { start_ms: 1234.0, bpm: 120.0 }]);

        let mut m = map(&[(500.0, 120.0)]);
        m.set_bpm_at(10510.0, 90.0).unwrap();
        assert_eq!(m, with_tempo());
    }

    #[test]
    fn set_bpm_insert_between() {
        let mut m = with_tempo();
        m.set_bpm_at(5000.0, 100.0).unwrap();
        assert_eq!(m.segments.len(), 3);
        assert_eq!(m.segments[1], Segment { start_ms: 5000.0, bpm: 100.0 });
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
        assert_eq!(m.segments, vec![Segment { start_ms: 500.0, bpm: 120.0 }]);
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
}
