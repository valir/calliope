use super::*;
use std::f64::consts::PI;

const RATES: [u32; 2] = [48_000, 44_100];

fn amp(dbfs: f64) -> f64 {
    10f64.powf(dbfs / 20.0)
}

/// Stereo (both channels equal, or the left only) 1 kHz sine segments `(dBFS peak, seconds)`
/// with continuous phase and no fades.
fn segments(rate: u32, segs: &[(f64, f64)], left_only: bool) -> Vec<f32> {
    let mut out = Vec::new();
    let mut n = 0usize;
    for &(db, secs) in segs {
        let frames = (secs * rate as f64).round() as usize;
        for _ in 0..frames {
            let v = (amp(db) * (2.0 * PI * 1000.0 * n as f64 / rate as f64).sin()) as f32;
            out.push(v);
            out.push(if left_only { 0.0 } else { v });
            n += 1;
        }
    }
    out
}

/// A sine of `freq` Hz, `peak` (linear) and `phase_deg`, with 20 ms raised-cosine fades.
fn faded_tone(rate: u32, freq: f64, peak: f64, phase_deg: f64, secs: f64) -> Vec<f32> {
    let frames = (secs * rate as f64) as usize;
    let fade = (0.02 * rate as f64) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let edge = n.min(frames - 1 - n);
        let w = if edge >= fade {
            1.0
        } else {
            0.5 - 0.5 * (PI * edge as f64 / fade as f64).cos()
        };
        let v =
            (w * peak * (2.0 * PI * freq * n as f64 / rate as f64 + phase_deg.to_radians()).sin())
                as f32;
        out.push(v);
        out.push(v);
    }
    out
}

fn measure(rate: u32, pcm: &[f32]) -> Measure {
    let mut m = Meter::new(rate).unwrap();
    m.add(pcm).unwrap();
    m.finish()
}

fn near(got: Option<f64>, want: f64, tol: f64, what: &str) {
    let g = got.unwrap_or_else(|| panic!("{what}: none"));
    eprintln!("loudness test {what}: measured {g:.4}, expected {want}");
    assert!((g - want).abs() <= tol, "{what}: {g} vs {want}");
}

#[test]
fn ebu_3341_loudness_cases() {
    #[allow(clippy::type_complexity)]
    let cases: [(&str, &[(f64, f64)]); 5] = [
        ("1", &[(-23.0, 20.0)]),
        ("2", &[(-33.0, 20.0)]),
        ("3", &[(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)]),
        (
            "4",
            &[
                (-72.0, 10.0),
                (-36.0, 10.0),
                (-23.0, 60.0),
                (-36.0, 10.0),
                (-72.0, 10.0),
            ],
        ),
        ("5", &[(-26.0, 20.0), (-20.0, 20.1), (-26.0, 20.0)]),
    ];
    let want = [-23.0, -33.0, -23.0, -23.0, -23.0];
    for rate in RATES {
        for ((name, segs), want) in cases.iter().zip(want) {
            let m = measure(rate, &segments(rate, segs, false));
            near(m.lufs, want, 0.1, &format!("case {name} @{rate}"));
        }
    }
    // the fixtures' rate
    let m = measure(22_050, &segments(22_050, &[(-23.0, 20.0)], false));
    near(m.lufs, -23.0, 0.1, "case 1 @22050");
}

#[test]
fn a_left_only_signal_is_3_lu_quieter() {
    for rate in RATES {
        let m = measure(rate, &segments(rate, &[(-23.0, 20.0)], true));
        near(m.lufs, -26.0, 0.1, &format!("left only @{rate}"));
    }
}

#[test]
fn ebu_3341_true_peak_cases() {
    // (case, frequency divisor of fs, peak, phase, expected dBTP)
    let cases = [
        ("15", 4.0, 0.5, 0.0, -6.0),
        ("16", 4.0, 0.5, 45.0, -6.0),
        ("17", 6.0, 0.5, 60.0, -6.0),
        ("18", 8.0, 0.5, 67.5, -6.0),
        ("19", 4.0, 1.41, 45.0, 3.0),
    ];
    for rate in RATES {
        for (name, div, peak, phase, want) in cases {
            let pcm = faded_tone(rate, rate as f64 / div, peak, phase, 3.0);
            let m = measure(rate, &pcm);
            near(
                m.true_peak,
                want,
                0.1,
                &format!("true peak case {name} @{rate}"),
            );
        }
    }
}

#[test]
fn silence_and_too_short_or_too_quiet_signals_have_no_loudness() {
    let m = measure(48_000, &vec![0.0f32; 48_000 * 2 * 2]);
    assert_eq!(
        m,
        Measure {
            lufs: None,
            true_peak: None
        }
    );
    let m = measure(48_000, &segments(48_000, &[(-80.0, 5.0)], false));
    assert_eq!(m.lufs, None);
    assert!(m.true_peak.is_some_and(|t| (t + 80.0).abs() < 0.3), "{m:?}");
    let m = measure(48_000, &segments(48_000, &[(-20.0, 0.3)], false));
    assert_eq!(m.lufs, None);
    assert!(m.true_peak.is_some());
}

#[test]
fn the_block_size_does_not_matter() {
    let pcm = segments(44_100, &[(-23.0, 20.0)], false);
    let whole = measure(44_100, &pcm);
    let mut m = Meter::new(44_100).unwrap();
    for c in pcm.chunks(4096 * 2) {
        m.add(c).unwrap();
    }
    let blocks = m.finish();
    assert!((whole.lufs.unwrap() - blocks.lufs.unwrap()).abs() < 1e-9);
    assert!((whole.true_peak.unwrap() - blocks.true_peak.unwrap()).abs() < 1e-9);
}

fn meas(l: Option<f64>, tp: f64) -> Measure {
    Measure {
        lufs: l,
        true_peak: Some(tp),
    }
}

#[test]
fn the_decision_table() {
    use Outcome::*;
    use Purpose::*;
    let s = |l: f64| Some(l);
    let table = [
        (meas(s(-20.0), -10.0), Save, Normalized, 6.0),
        (meas(s(-20.0), -3.0), Save, Limited, 2.0),
        (meas(s(-14.05), -4.0), Normalize, AtTarget, 0.0),
        (meas(s(-14.05), -4.0), Save, Normalized, 0.05),
        (meas(s(-15.6), -1.05), Normalize, AtCeiling, 0.0),
        (meas(s(-14.0), 0.5), Normalize, Limited, -1.5),
        (meas(s(-10.0), -0.5), Normalize, Normalized, -4.0),
        (meas(None, -3.0), Save, TooQuiet, 0.0),
        (meas(None, -3.0), Normalize, TooQuiet, 0.0),
        (meas(s(-14.1), -10.0), Normalize, AtTarget, 0.0),
        (meas(s(-14.11), -10.0), Normalize, Normalized, 0.11),
    ];
    for (m, p, outcome, gain) in table {
        let d = decide(m, p);
        assert_eq!(d.outcome, outcome, "{m:?} {p:?}");
        assert!(
            (d.gain_db - gain).abs() < 1e-9,
            "{m:?} {p:?}: {}",
            d.gain_db
        );
    }
    let d = decide(
        Measure {
            lufs: Some(-20.0),
            true_peak: None,
        },
        Save,
    );
    assert_eq!((d.outcome, d.gain_db), (TooQuiet, 0.0));
}

#[test]
fn gain_factor_and_results() {
    assert!((db_to_gain(6.0) - 1.995262).abs() < 1e-6);
    assert_eq!(db_to_gain(0.0), 1.0);
    let m = meas(Some(-20.0), -10.0);
    let d = decide(m, Purpose::Save);
    let r = result_of(m, d);
    assert_eq!(
        (r.lufs, r.true_peak, r.gain_db),
        (Some(-14.0), Some(-4.0), 6.0)
    );
    assert_eq!(
        (r.target, r.ceiling, r.outcome),
        (-14.0, -1.0, Outcome::Normalized)
    );
    let q = Measure {
        lufs: None,
        true_peak: None,
    };
    let r = result_of(q, decide(q, Purpose::Save));
    assert_eq!((r.lufs, r.true_peak, r.gain_db), (None, None, 0.0));
    let rec = record_of(m, d, [0xab; 16]);
    assert_eq!(
        (rec.lufs, rec.true_peak, rec.gain_db),
        (Some(-14.0), Some(-4.0), 6.0)
    );
    assert_eq!((rec.target, rec.ceiling), (-14.0, -1.0));
    assert_eq!(rec.md5, "ab".repeat(16));
}

#[test]
fn a_measured_tone_normalizes_to_the_target() {
    // end to end: measure, decide, scale, measure again
    let pcm = segments(22_050, &[(-20.0, 6.0)], false);
    let m = measure(22_050, &pcm);
    let d = decide(m, Purpose::Save);
    assert_eq!(d.outcome, Outcome::Normalized);
    let g = db_to_gain(d.gain_db);
    let scaled: Vec<f32> = pcm.iter().map(|v| v * g).collect();
    near(measure(22_050, &scaled).lufs, -14.0, 0.05, "scaled tone");
}
