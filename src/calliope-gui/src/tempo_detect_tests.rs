use super::*;
use crate::tempo_map::fine_ms;

struct Lcg(u64);
impl Lcg {
    /// Uniform in -1..1.
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

/// Beats (seconds) from `(bars, bpm)` sections starting at 0.5 s, 4 beats a bar, the last beat
/// closing the final bar. Downbeats every 4th beat.
fn make(sections: &[(usize, f64)], jitter_ms: f64, seed: u64) -> (Vec<f64>, Vec<f64>) {
    let mut rng = Lcg(seed);
    let (mut beats, mut down) = (Vec::new(), Vec::new());
    let mut t = 0.5;
    for &(bars, bpm) in sections {
        for _ in 0..bars * 4 {
            let j = rng.next() * jitter_ms / 1000.0;
            if beats.len() % 4 == 0 {
                down.push(t + j);
            }
            beats.push(t + j);
            t += 60.0 / bpm;
        }
    }
    beats.push(t + rng.next() * jitter_ms / 1000.0);
    (beats, down)
}

fn bpms(d: &Detected) -> Vec<f64> {
    d.map.segments.iter().map(|s| s.bpm).collect()
}

/// Same number of sections, each BPM within `tol` of the expected one.
fn assert_bpms(d: &Detected, want: &[f64], tol: f64, what: &str) {
    let got = bpms(d);
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol), "{what}: {got:?} vs {want:?}");
}

fn on_fine_grid(d: &Detected) {
    for k in 1..d.map.segments.len() {
        let (l, s) = (&d.map.segments[k - 1], &d.map.segments[k]);
        let steps = (s.start_ms - l.start_ms) / fine_ms(l.bpm);
        assert!((steps - steps.round()).abs() * fine_ms(l.bpm) < 1e-6, "change {k} off the grid: {steps}");
        assert_eq!(s.detected_ms, Some(s.start_ms));
    }
    assert_eq!(d.map.segments[0].detected_ms, None);
}

fn run(sections: &[(usize, f64)], jitter: f64, seed: u64) -> Detected {
    let (b, d) = make(sections, jitter, seed);
    detect(&b, &d, 4).unwrap()
}

#[test]
fn steady_120_with_and_without_jitter() {
    for (jitter, seed) in [(0.0, 1), (10.0, 2), (10.0, 3)] {
        let d = run(&[(40, 120.0)], jitter, seed);
        assert_eq!(bpms(&d), vec![120.0]);
        assert!((d.map.segments[0].start_ms - 500.0).abs() < 5.0, "{}", d.map.segments[0].start_ms);
        assert_eq!(d.bar_beats, 4);
    }
}

#[test]
fn stub_ok_pattern_is_exact() {
    let mut beats: Vec<f64> = (0..12).map(|i| 0.5 + i as f64 * 0.5).collect();
    beats.extend((0..15).map(|i| 6.5 + i as f64 * 60.0 / 90.0));
    let down: Vec<f64> = beats.iter().step_by(4).copied().collect();
    let d = detect(&beats, &down, 4).unwrap();
    assert_eq!(
        d.map.segments,
        vec![
            Segment { start_ms: 500.0, bpm: 120.0, detected_ms: None },
            Segment { start_ms: 6500.0, bpm: 90.0, detected_ms: Some(6500.0) },
        ]
    );
}

#[test]
fn stub_steady_and_bar_end_patterns() {
    let steady: Vec<f64> = (0..28).map(|i| 0.5 + i as f64 * 0.5).collect();
    let d = detect(&steady, &steady, 4).unwrap();
    assert_eq!(bpms(&d), vec![120.0]);
    // 120 BPM with the last full bar at 60 BPM.
    let d = run(&[(7, 120.0), (1, 60.0)], 0.0, 1);
    assert_eq!(bpms(&d), vec![120.0, 60.0]);
    on_fine_grid(&d);
    assert_eq!(d.map.segments[1].start_ms, 14500.0);
}

#[test]
fn three_tempos_each_change_on_the_fine_grid() {
    for jitter in [0.0, 8.0] {
        let d = run(&[(12, 120.0), (10, 90.0), (14, 140.0)], jitter, 7);
        assert_eq!(bpms(&d), vec![120.0, 90.0, 140.0], "jitter {jitter}");
        on_fine_grid(&d);
        // The changes are within a few ms of the true ones.
        let truth = [500.0 + 12.0 * 2000.0, 500.0 + 24000.0 + 10.0 * 4.0 * 60000.0 / 90.0];
        for (s, t) in d.map.segments[1..].iter().zip(truth) {
            assert!((s.start_ms - t).abs() < 15.0, "{} vs {t}", s.start_ms);
        }
    }
}

#[test]
fn one_bar_sections() {
    for (jitter, seed) in [(0.0, 1), (10.0, 4), (10.0, 5)] {
        let d = run(&[(20, 120.0), (1, 100.0), (20, 120.0)], jitter, seed);
        assert_bpms(&d, &[120.0, 100.0, 120.0], if jitter == 0.0 { 0.05 } else { 1.0 }, "mid");
        on_fine_grid(&d);
        let d = run(&[(30, 120.0), (1, 60.0)], jitter, seed);
        assert_bpms(&d, &[120.0, 60.0], if jitter == 0.0 { 0.05 } else { 1.0 }, "end");
        on_fine_grid(&d);
        assert!((d.sections[1].start_ms - (500.0 + 30.0 * 2000.0)).abs() < 15.0);
    }
}

#[test]
fn one_bar_three_percent_off_is_no_section() {
    for jitter in [0.0, 5.0] {
        let d = run(&[(20, 120.0), (1, 120.0 / 1.03), (20, 120.0)], jitter, 9);
        assert_bpms(&d, &[120.0], 0.3, "3 %");
    }
}

#[test]
fn fills_and_glitches_make_no_section() {
    // A 4-beat fill of jitter.
    let (mut b, d) = make(&[(30, 120.0)], 0.0, 1);
    for (i, x) in b.iter_mut().enumerate().skip(40).take(4) {
        *x += [0.03, -0.04, 0.05, -0.03][i - 40];
    }
    assert_eq!(bpms(&detect(&b, &d, 4).unwrap()), vec![120.0]);
    // An isolated missed beat.
    let (mut b, d) = make(&[(30, 120.0)], 3.0, 2);
    b.remove(50);
    let r = detect(&b, &d, 4).unwrap();
    assert_eq!(bpms(&r), vec![120.0]);
    // An isolated double beat.
    let (mut b, d) = make(&[(30, 120.0)], 3.0, 3);
    let mid = (b[50] + b[51]) / 2.0;
    b.insert(51, mid);
    assert_eq!(bpms(&detect(&b, &d, 4).unwrap()), vec![120.0]);
}

#[test]
fn four_interval_half_time_run_is_not_repaired() {
    // Beats at 60 BPM for one bar: 4 intervals of 1 s inside a 120 BPM song.
    let d = run(&[(15, 120.0), (1, 60.0), (15, 120.0)], 2.0, 11);
    assert_eq!(bpms(&d), vec![120.0, 60.0, 120.0]);
}

#[test]
fn tempo_folding_and_anchor() {
    let d = run(&[(40, 350.0)], 0.0, 1);
    assert_eq!(bpms(&d), vec![175.0]);
    // No downbeats: the anchor is the first beat.
    let (b, _) = make(&[(30, 120.0)], 0.0, 1);
    let d = detect(&b, &[], 4).unwrap();
    assert!((d.map.segments[0].start_ms - 500.0).abs() < 1e-6);
    assert_eq!(d.bar_beats, 0);
    // The anchor is the first downbeat, not the first beat.
    let down: Vec<f64> = b.iter().skip(1).step_by(4).copied().collect();
    let d = detect(&b, &down, 4).unwrap();
    assert!((d.map.segments[0].start_ms - 1000.0).abs() < 1e-6);
}

#[test]
fn unsorted_duplicates_and_too_few() {
    let (mut b, d) = make(&[(10, 120.0)], 0.0, 1);
    let want = detect(&b, &d, 4).unwrap().map;
    b.reverse();
    let dup = b[3];
    b.push(dup + 0.0004);
    b.push(f64::NAN);
    b.push(-1.0);
    assert_eq!(detect(&b, &d, 4).unwrap().map, want);
    let err = detect(&[0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5], &[], 4).unwrap_err();
    assert_eq!(err, NO_BEAT_ERROR);
    assert!(detect(&[], &[], 4).is_err());
}

#[test]
fn long_drift_from_the_bpm_rounding_stays_small() {
    // 121.04 BPM for 4 minutes.
    let n = (240.0 * 121.04 / 60.0) as usize;
    let p = 60.0 / 121.04;
    let beats: Vec<f64> = (0..n).map(|i| 0.5 + i as f64 * p).collect();
    let down: Vec<f64> = beats.iter().step_by(4).copied().collect();
    let d = detect(&beats, &down, 4).unwrap();
    assert_eq!(bpms(&d), vec![121.0]);
    let (a, grid) = (d.map.segments[0].start_ms, 60000.0 / 121.0);
    for (i, b) in beats.iter().enumerate() {
        assert!((a + i as f64 * grid - b * 1000.0).abs() < 60.0, "beat {i}");
    }
}

#[test]
fn long_input_is_fast() {
    let (b, d) = make(&[(300, 120.0), (300, 100.0), (300, 140.0), (225, 90.0)], 4.0, 21);
    assert!(b.len() >= 4500);
    let t = std::time::Instant::now();
    let r = detect(&b, &d, 4).unwrap();
    assert!(t.elapsed().as_millis() < 200, "{:?}", t.elapsed());
    assert_eq!(bpms(&r), vec![120.0, 100.0, 140.0, 90.0]);
}

#[test]
fn one_bar_five_percent_off_with_ten_ms_jitter_is_found() {
    for seed in 1..=10 {
        let d = run(&[(20, 120.0), (1, 120.0 / 1.05), (20, 120.0)], 10.0, seed);
        assert_eq!(d.map.segments.len(), 3, "seed {seed}");
        on_fine_grid(&d);
    }
}
