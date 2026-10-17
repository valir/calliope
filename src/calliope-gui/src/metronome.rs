//! Metronome click sounds, count-in plan and beat placement. Pure logic, no IO.

use std::f32::consts::TAU;

use crate::tempo_map::TempoMap;

pub(crate) const CLICK_PEAK: f32 = 0.5;
/// One tone for every beat (owner: the lower of the two earlier tones; no accent on the bar start).
pub(crate) const CLICK_HZ: f32 = 1500.0;
const CLICK_SECONDS: f32 = 0.030;
/// Decay time constant: `exp(-30 ms / tau)` is about 0.0067, so the click is below 0.01 at 30 ms.
const DECAY_TAU: f32 = 0.006;

/// The pre-rendered click sound at the output rate.
#[derive(Debug, Clone)]
pub(crate) struct Click {
    samples: Vec<f32>,
}

fn render(hz: f32, rate: u32) -> Vec<f32> {
    let n = ((CLICK_SECONDS * rate as f32).round() as usize).max(1);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            (TAU * hz * t).sin() * (-t / DECAY_TAU).exp()
        })
        .collect();
    let peak = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        for s in &mut v {
            *s *= CLICK_PEAK / peak;
        }
    }
    v
}

impl Click {
    pub(crate) fn new(rate: u32) -> Self {
        Click { samples: render(CLICK_HZ, rate) }
    }

    pub(crate) fn samples(&self) -> &[f32] {
        &self.samples
    }
}

/// Output frames of the three audible count-in clicks and of the audio start (the silent 4th beat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CountIn {
    pub clicks: [u64; 3],
    pub start: u64,
}

pub(crate) fn count_in_plan(bpm: f64, rate: u32) -> CountIn {
    let b = (rate as f64 * 60.0 / bpm).round() as u64;
    CountIn {
        clicks: [0, b, 2 * b],
        start: 3 * b,
    }
}

/// Beats (source frame, accent) that fall inside the output block starting at source frame
/// `block_start` and `block_len` output frames long, as (output offset, accent). At `speed`
/// the block covers `block_len * speed` source frames. `beats` must be sorted.
#[allow(dead_code)] // the engine places clicks itself; kept for the tests
pub(crate) fn place(
    beats: &[(u64, bool)],
    block_start: u64,
    block_len: usize,
    speed: f64,
) -> impl Iterator<Item = (usize, bool)> + '_ {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    let first = beats.partition_point(|b| b.0 < block_start);
    beats[first..]
        .iter()
        .map(move |&(f, a)| ((((f - block_start) as f64) / speed).ceil() as usize, a))
        .take_while(move |&(off, _)| off < block_len)
}

/// The clicks to sound, in ms, sorted: the beats of sections whose metronome is on, with each section's
/// count-in (three clicks at its tempo before its first beat) replacing the clicks before it. `wrap` is a
/// repeating loop (start, end) in ms: a count-in whose span the loop ends in is skipped.
#[allow(dead_code)] // used by the Player session in task 16
pub(crate) fn section_clicks(map: &TempoMap, dur_ms: f64, wrap: Option<(f64, f64)>) -> Vec<f64> {
    const EPS: f64 = 1e-6;
    let mut clicks: Vec<f64> = map
        .beats_in(0.0, dur_ms)
        .into_iter()
        .filter(|&(t, _)| map.segment_at(t).is_none_or(|k| map.segments[k].meta.metronome))
        .map(|(t, _)| t)
        .collect();
    for seg in map.segments.iter().skip(1) {
        if !seg.meta.count_in {
            continue;
        }
        let b = 60000.0 / seg.bpm;
        let Some(&(first, _)) = map.beats_in(seg.start_ms - EPS, dur_ms).first() else {
            continue;
        };
        if first >= dur_ms {
            continue;
        }
        if let Some((_, le)) = wrap {
            if first - 3.5 * b < le && le <= first + EPS {
                continue;
            }
        }
        clicks.retain(|&t| !(t > first - 3.5 * b && t < first - EPS));
        clicks.extend([3.0, 2.0, 1.0].iter().map(|n| first - n * b).filter(|&t| t >= 0.0));
    }
    clicks.sort_by(f64::total_cmp);
    clicks.dedup_by(|a, b| (*a - *b).abs() < EPS);
    clicks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goertzel(x: &[f32], hz: f32, rate: u32) -> f32 {
        let w = TAU * hz / rate as f32;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &v in x {
            let s = v + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - c * s1 * s2
    }

    #[test]
    fn count_in_at_120_bpm() {
        let p = count_in_plan(120.0, 22050);
        assert_eq!(p.clicks, [0, 11025, 22050]);
        assert_eq!(p.start, 33075);
    }

    #[test]
    fn click_peak_and_decay() {
        let rate = 22050;
        let c = Click::new(rate);
        let s = c.samples();
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 1e-4);
        let n30 = (0.030 * rate as f32).round() as usize;
        assert!(s.len() <= n30 + 1);
        assert!(s.iter().skip(n30.saturating_sub(1)).all(|v| v.abs() < 0.01));
    }

    #[test]
    fn click_is_the_one_lower_tone() {
        let rate = 22050;
        let s = Click::new(rate).samples().to_vec();
        assert!(goertzel(&s, 1500.0, rate) > 3.0 * goertzel(&s, 2000.0, rate));
    }

    #[test]
    fn place_only_beats_in_block() {
        let beats = [(0, true), (100, false), (200, false), (300, true)];
        let got: Vec<_> = place(&beats, 100, 100, 1.0).collect();
        assert_eq!(got, vec![(0, false)]);
        let got: Vec<_> = place(&beats, 50, 200, 1.0).collect();
        assert_eq!(got, vec![(50, false), (150, false)]);
        assert_eq!(place(&beats, 301, 100, 1.0).count(), 0);
    }

    #[test]
    fn place_scales_with_speed() {
        let beats = [(100, true), (300, false)];
        // speed 2: 100 output frames cover 200 source frames
        let got: Vec<_> = place(&beats, 0, 100, 2.0).collect();
        assert_eq!(got, vec![(50, true)]);
    }

    use crate::tempo_map::{SectionMeta, Segment};

    fn seg(start: f64, bpm: f64, on_beat: bool, metronome: bool, count_in: bool) -> Segment {
        Segment {
            start_ms: start,
            bpm,
            detected_ms: None,
            on_beat,
            meta: SectionMeta { name: String::new(), metronome, count_in },
        }
    }

    fn tmap(segs: Vec<Segment>) -> TempoMap {
        TempoMap { beats_per_bar: 4, segments: segs }
    }

    /// 120 BPM from 500, 93 BPM detected at 8500 with a count-in.
    fn ts11(prev_on: bool) -> TempoMap {
        let mut s1 = seg(8500.0, 93.0, false, true, true);
        s1.detected_ms = Some(8500.0);
        tmap(vec![seg(500.0, 120.0, false, prev_on, false), s1])
    }

    fn has(c: &[f64], v: f64) -> bool {
        c.iter().any(|t| (t - v).abs() < 1e-3)
    }

    fn near(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-3, "{got:?} vs {want:?}");
        }
    }

    fn in_range(c: &[f64], a: f64, b: f64) -> Vec<f64> {
        c.iter().copied().filter(|&t| t > a && t < b).collect()
    }

    fn plain(m: &TempoMap, dur: f64) -> Vec<f64> {
        m.beats_in(0.0, dur).into_iter().map(|b| b.0).collect()
    }

    #[test]
    fn clicks_default_meta_equal_beats() {
        let m = tmap(vec![seg(500.0, 120.0, false, true, false), seg(8500.0, 93.0, true, true, false)]);
        assert_eq!(section_clicks(&m, 20000.0, None), plain(&m, 20000.0));
        assert!(section_clicks(&tmap(vec![]), 20000.0, None).is_empty());
    }

    #[test]
    fn clicks_section_off() {
        let m = tmap(vec![seg(500.0, 120.0, false, true, false), seg(4500.0, 120.0, true, false, false)]);
        let got = section_clicks(&m, 10000.0, None);
        let want: Vec<f64> = plain(&m, 10000.0).into_iter().filter(|&t| t < 4500.0 - 1e-6).collect();
        assert!(!got.is_empty());
        assert_eq!(got, want);
        // the beats before the first start belong to section 0
        let m = tmap(vec![seg(500.0, 120.0, false, false, false)]);
        assert!(section_clicks(&m, 5000.0, None).is_empty());
    }

    #[test]
    fn clicks_ts11_count_in() {
        let c = section_clicks(&ts11(true), 12000.0, None);
        assert!(has(&c, 6000.0));
        near(&in_range(&c, 6241.94, 8500.0 - 1e-6), &[6564.516, 7209.677, 7854.839]);
        assert!(has(&c, 8500.0) && has(&c, 9145.161));
        // previous section off: the same three clicks, nothing else
        let c = section_clicks(&ts11(false), 12000.0, None);
        near(&in_range(&c, 0.0, 8500.0 - 1e-6), &[6564.516, 7209.677, 7854.839]);
    }

    #[test]
    fn clicks_typed_change_anchors_on_first_beat() {
        let m = tmap(vec![seg(500.0, 120.0, false, true, false), seg(8530.0, 100.0, false, true, true)]);
        let first = m.beats_in(8530.0 - 1e-6, 20000.0)[0].0;
        let c = section_clicks(&m, 20000.0, None);
        assert!(has(&c, first));
        for n in [1.0, 2.0, 3.0] {
            assert!(has(&c, first - n * 600.0), "{first} {c:?}");
        }
    }

    #[test]
    fn clicks_short_previous_section() {
        let m = tmap(vec![
            seg(0.0, 120.0, false, true, false),
            seg(4000.0, 120.0, true, true, false),
            seg(5000.0, 60.0, true, true, true),
        ]);
        let c = section_clicks(&m, 8000.0, None);
        near(&in_range(&c, 1400.0, 5000.0 - 1e-6), &[1500.0, 2000.0, 3000.0, 4000.0]);
        assert!(has(&c, 5000.0));
    }

    #[test]
    fn clicks_overlapping_count_ins_later_wins() {
        let m = tmap(vec![
            seg(0.0, 120.0, false, true, false),
            seg(4000.0, 120.0, true, true, true),
            seg(5000.0, 60.0, true, true, true),
        ]);
        let c = section_clicks(&m, 8000.0, None);
        near(&in_range(&c, 1400.0, 5000.0 - 1e-6), &[1500.0, 2000.0, 3000.0, 4000.0]);
    }

    #[test]
    fn clicks_wrap_loop_skips_count_in() {
        let m = ts11(true);
        let c = section_clicks(&m, 12000.0, Some((500.0, 8500.0)));
        assert_eq!(c, plain(&m, 12000.0));
        let c = section_clicks(&m, 12000.0, Some((500.0, 6000.0)));
        near(&in_range(&c, 6241.94, 8500.0 - 1e-6), &[6564.516, 7209.677, 7854.839]);
    }

    #[test]
    fn clicks_edges_and_order() {
        // count-in clicks before 0 ms are dropped
        let m = tmap(vec![seg(0.0, 120.0, false, true, false), seg(800.0, 120.0, true, true, true)]);
        let c = section_clicks(&m, 4000.0, None);
        assert!(c.iter().all(|&t| t >= 0.0));
        near(&c[..2], &[300.0, 800.0]);
        // a section starting at or after dur is ignored
        let m = ts11(true);
        assert_eq!(section_clicks(&m, 8500.0, None), plain(&m, 8500.0));
        for map in [ts11(true), ts11(false), m] {
            let c = section_clicks(&map, 12000.0, None);
            assert!(c.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
