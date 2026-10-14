//! Beats to tempo sections (plan 2.6). Pure logic, no IO.
//!
//! `detect` cleans the tracker's beats (isolated glitches only), cuts them into sections with an
//! optimal least-squares segmentation (dynamic programming with PELT-style pruning; sections are
//! at least one bar of 4 intervals (3 for the last one, which no beat follows) and share their
//! change beat), merges neighbours that differ by
//! no more than `CHANGE`, and turns the sections into a `TempoMap`: the first segment is the
//! anchor, every later one a detected change snapped to 1/64 beat of the left section's tempo.
//!
//! Two rules clean the segmentation (owner change 1, after trying a real song), applied one change
//! at a time with a re-segmentation after each until nothing changes:
//! 1. glitch sections: a run of short sections (< 2 bars) between agreeing neighbours whose beats
//!    fit the left neighbour's beat grid (extra beats = subdivisions, or missing beats), or a lone
//!    one whose beats are noise, is replaced by grid beats and so merges into its neighbours;
//! 2. octave errors: a section at about 2x or 0.5x the tempo of its left neighbour (the right one
//!    for the first section) gets its beats re-spaced at the neighbour's tempo.
//!
//! Finally each change is moved back over lead-in beats that already sit on the right grid.
//!
//! Final `PENALTY` = 30 (the plan's start value; all tests of this module pass with it).

#![allow(dead_code)] // used by the detection job (later task)

use crate::tempo_map::{fine_ms, Segment, TempoMap, DEFAULT_BEATS_PER_BAR, MAX_BPM, MIN_BPM};

const MIN_BEATS: usize = 8;
const MIN_SECTION_BEATS: usize = 4;
const CHANGE: f64 = 0.04;
const PENALTY: f64 = 30.0;
const MAX_SECTIONS: usize = 64;
const GLITCH_TOL: f64 = 0.15;
/// An octave error is a tempo of 2x or 0.5x within this much. Twice `CHANGE`: the tracker's
/// subdivision runs wobble (a 5 s run at 176.7 x 2 BPM next to 185 BPM is 5 % off an exact octave).
const OCTAVE_TOL: f64 = 2.0 * CHANGE;
const MAX_RULE_ROUNDS: usize = 200;

pub(crate) const NO_BEAT_ERROR: &str =
    "No steady beat was found in this track; set the tempo by hand";

/// One detected section, for the waveform labels and the log.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SectionInfo {
    pub start_ms: f64,
    pub end_ms: f64,
    pub bpm: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct Detected {
    pub map: TempoMap,
    pub sections: Vec<SectionInfo>,
    /// Tracker jitter estimate (ms), for the log line.
    pub sigma_ms: f64,
    /// The tracker's most frequent bar length (beats between downbeats), 0 when unknown.
    pub bar_beats: usize,
}

/// Drop invalid values, sort, drop duplicates within 1 ms; seconds in, ms out.
fn clean(values: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite() && *x >= 0.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<f64> = Vec::with_capacity(v.len());
    for x in v {
        let x = x * 1000.0;
        if out.last().is_none_or(|l| x - l >= 1.0) {
            out.push(x);
        }
    }
    out
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

/// The 6 neighbours agree within the tolerance; returns their median.
fn agreed(nb: &[f64]) -> Option<f64> {
    let m = median(nb);
    nb.iter().all(|x| (x / m - 1.0).abs() <= GLITCH_TOL).then_some(m)
}

/// Repair isolated missed beats (interval ~ 2 or 3 x the local one) and double beats (two
/// intervals of ~ 0.5 x). Beats in ms.
fn repair(beats: &[f64]) -> Vec<f64> {
    let d: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    let n = d.len();
    let mut insert: Vec<(usize, usize)> = Vec::new(); // (interval index, k)
    let mut remove: Vec<usize> = Vec::new(); // beat index
    let mut i = 3;
    while i + 3 < n {
        let nb = [d[i - 3], d[i - 2], d[i - 1], d[i + 1], d[i + 2], d[i + 3]];
        if let Some(m) = agreed(&nb) {
            if let Some(k) = [2usize, 3].into_iter().find(|&k| (d[i] / (k as f64 * m) - 1.0).abs() <= GLITCH_TOL) {
                insert.push((i, k));
                i += 4;
                continue;
            }
        }
        if i + 4 < n {
            let nb = [d[i - 3], d[i - 2], d[i - 1], d[i + 2], d[i + 3], d[i + 4]];
            if let Some(m) = agreed(&nb) {
                let sum = (d[i] + d[i + 1]) / m - 1.0;
                if sum.abs() <= GLITCH_TOL && d[i] >= 0.2 * m && d[i + 1] >= 0.2 * m {
                    remove.push(i + 1);
                    i += 5;
                    continue;
                }
            }
        }
        i += 1;
    }
    let mut out = Vec::with_capacity(beats.len() + insert.len());
    for (j, &b) in beats.iter().enumerate() {
        if !remove.contains(&j) {
            out.push(b);
        }
        if let Some(&(_, k)) = insert.iter().find(|(ii, _)| *ii == j) {
            for q in 1..k {
                out.push(b + d[j] * q as f64 / k as f64);
            }
        }
    }
    out
}

/// Prefix sums of (x = index, y = time detrended by the global period), for O(1) line fits.
struct Fit {
    sx: Vec<f64>,
    sy: Vec<f64>,
    sxx: Vec<f64>,
    sxy: Vec<f64>,
    syy: Vec<f64>,
}

impl Fit {
    fn new(t: &[f64], period: f64) -> Fit {
        let mut f = Fit { sx: vec![0.0], sy: vec![0.0], sxx: vec![0.0], sxy: vec![0.0], syy: vec![0.0] };
        for (i, &ti) in t.iter().enumerate() {
            let x = i as f64;
            let y = ti - t[0] - x * period;
            f.sx.push(f.sx[i] + x);
            f.sy.push(f.sy[i] + y);
            f.sxx.push(f.sxx[i] + x * x);
            f.sxy.push(f.sxy[i] + x * y);
            f.syy.push(f.syy[i] + y * y);
        }
        f
    }

    /// Sum of squared residuals of the line through beats a..=b.
    fn cost(&self, a: usize, b: usize) -> f64 {
        let m = (b - a + 1) as f64;
        let (sx, sy) = (self.sx[b + 1] - self.sx[a], self.sy[b + 1] - self.sy[a]);
        let sxx = self.sxx[b + 1] - self.sxx[a] - sx * sx / m;
        let sxy = self.sxy[b + 1] - self.sxy[a] - sx * sy / m;
        let syy = self.syy[b + 1] - self.syy[a] - sy * sy / m;
        (syy - sxy * sxy / sxx).max(0.0)
    }
}

/// Least-squares period (ms per beat index) over beats a..=b.
fn slope(t: &[f64], a: usize, b: usize) -> f64 {
    let m = (b - a + 1) as f64;
    let mx = (a + b) as f64 / 2.0;
    let my = t[a..=b].iter().sum::<f64>() / m;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, &ti) in t[a..=b].iter().enumerate() {
        let dx = (a + i) as f64 - mx;
        sxy += dx * (ti - my);
        sxx += dx * dx;
    }
    sxy / sxx
}

/// Optimal partition; returns the boundary beat indices (first 0, last n-1).
fn segment(t: &[f64], sigma: f64) -> Vec<usize> {
    let n = t.len();
    let period = (t[n - 1] - t[0]) / (n - 1) as f64;
    let fit = Fit::new(t, period);
    let pen = PENALTY * sigma * sigma;
    let mut f = vec![f64::INFINITY; n];
    let mut back = vec![0usize; n];
    f[0] = -pen;
    let mut cands: Vec<usize> = Vec::new();
    for j in MIN_SECTION_BEATS..n {
        let new = j - MIN_SECTION_BEATS;
        if f[new].is_finite() {
            cands.push(new);
        }
        // The last section may be a single bar of 3 intervals: no beat follows it.
        let last_bar = (j == n - 1 && j > MIN_SECTION_BEATS - 1).then(|| j - (MIN_SECTION_BEATS - 1)).filter(|&i| f[i].is_finite());
        if let Some(i) = last_bar {
            cands.push(i);
        }
        let mut best = f64::INFINITY;
        for &i in &cands {
            let v = f[i] + fit.cost(i, j) + pen;
            if v < best {
                best = v;
                back[j] = i;
            }
        }
        f[j] = best;
        // PELT pruning; the shared change beat breaks strict additivity, so keep a margin.
        cands.retain(|&i| f[i] + fit.cost(i, j) <= best);
    }
    let mut bounds = vec![n - 1];
    while *bounds.last().unwrap() != 0 {
        bounds.push(back[*bounds.last().unwrap()]);
    }
    bounds.reverse();
    bounds
}

/// Merge neighbours differing by <= CHANGE (and the closest pairs above MAX_SECTIONS).
fn merge_close(t: &[f64], bounds: &mut Vec<usize>) {
    loop {
        let m = bounds.len() - 1;
        if m < 2 {
            return;
        }
        let periods: Vec<f64> = (0..m).map(|k| slope(t, bounds[k], bounds[k + 1])).collect();
        let (mut at, mut diff) = (0, f64::INFINITY);
        for k in 0..m - 1 {
            let r = (periods[k] / periods[k + 1]).max(periods[k + 1] / periods[k]) - 1.0;
            if r < diff {
                diff = r;
                at = k;
            }
        }
        if diff <= CHANGE || m > MAX_SECTIONS {
            bounds.remove(at + 1);
        } else {
            return;
        }
    }
}

/// Fold into MIN_BPM..=MAX_BPM by doubling or halving; returns (bpm rounded to 0.1, factor f with
/// grid period = f x beat period).
fn quantise(period_ms: f64) -> (f64, f64) {
    let (mut bpm, mut f) = (60000.0 / period_ms, 1.0);
    while bpm > MAX_BPM {
        bpm /= 2.0;
        f *= 2.0;
    }
    while bpm < MIN_BPM {
        bpm *= 2.0;
        f /= 2.0;
    }
    ((bpm * 10.0).round() / 10.0, f)
}

fn round_ms(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Intercept of the fixed-slope fit over beats a..=b, moved to beat `at`.
fn intercept(t: &[f64], a: usize, b: usize, at: usize, bpm: f64, f: f64) -> f64 {
    let grid = 60000.0 / bpm;
    let sum: f64 = (a..=b).map(|i| t[i] - (i as f64 - at as f64) / f * grid).sum();
    sum / (b - a + 1) as f64
}

/// Most frequent number of beats between consecutive downbeats (0 when unknown).
fn bar_length(t: &[f64], down: &[f64]) -> usize {
    let idx: Vec<usize> = down.iter().map(|&x| t.partition_point(|&b| b < x - 1.0)).filter(|&i| i < t.len()).collect();
    let mut counts = std::collections::BTreeMap::new();
    for w in idx.windows(2) {
        if w[1] > w[0] {
            *counts.entry(w[1] - w[0]).or_insert(0usize) += 1;
        }
    }
    counts.into_iter().max_by_key(|&(len, c)| (c, std::cmp::Reverse(len))).map_or(0, |(len, _)| len)
}

/// Largest ratio between two periods, minus 1.
fn ratio_diff(a: f64, b: f64) -> f64 {
    (a / b).max(b / a) - 1.0
}

/// Neighbouring tempos agree: equal within `CHANGE`, or (the right one being an octave error that
/// rule 2 folds next) an octave apart within `OCTAVE_TOL`.
fn agree(left: f64, right: f64) -> bool {
    ratio_diff(left, right) <= CHANGE || ratio_diff(left, 2.0 * right) <= OCTAVE_TOL || ratio_diff(left, right / 2.0) <= OCTAVE_TOL
}

/// Sections shorter than this many bars are candidates for the glitch rule.
const GLITCH_BARS: f64 = 2.0;
/// Share of the grid beats that must have a detected beat nearby (extra beats) for a section to
/// be a glitch.
/// Longest run of consecutive short sections absorbed together.
const MAX_RUN: usize = 4;
const GLITCH_SHARE: f64 = 0.75;
/// A section whose rms fit residual is this many sigma is too irregular to be a real tempo
/// (a steady section sits near 0.8 sigma; the Deja-Vu fill bursts at 1.7 to 2.9).
const IRREGULAR_SIGMA: f64 = 1.75;

/// Rule 1: a short section between agreeing neighbours that fits the left neighbour's beat grid
/// (or has no regular beats of its own) is replaced by grid beats, so the next segmentation
/// merges it into its neighbours. Returns true when `t` was changed.
fn absorb_glitch(t: &mut Vec<f64>, bounds: &[usize], sigma: f64, beats_per_bar: f64) -> bool {
    let m = bounds.len() - 1;
    // A run of up to MAX_RUN short sections between agreeing neighbours (a fill burst is often cut
    // into several pieces); the run at the very end only needs the left neighbour.
    for k in 1..m {
        let (al, a) = (bounds[k - 1], bounds[k]);
        let pl = slope(t, al, a);
        for run in 1..=MAX_RUN.min(m - k) {
            let b = bounds[k + run];
            if t[b] - t[bounds[k + run - 1]] >= GLITCH_BARS * beats_per_bar * pl {
                break; // this piece is long: no run reaching it can be a glitch
            }
            let agrees = bounds.get(k + run + 1).is_none_or(|&br| agree(pl, slope(t, b, br)));
            let fit = Fit::new(&t[a..=b], slope(t, a, b));
            // A lone section whose beats are noise may sit between neighbours that drifted apart
            // (up to 2 x CHANGE); the drift then moves to the section's right edge.
            let drift = bounds.get(k + run + 1).is_some_and(|&br| ratio_diff(pl, slope(t, b, br)) <= OCTAVE_TOL);
            let irregular = run == 1 && (agrees || drift) && (fit.cost(0, b - a) / (b - a + 1) as f64).sqrt() > IRREGULAR_SIGMA * sigma;
            if !agrees && !irregular {
                continue;
            }
            // Differing neighbours: space the beats evenly at the tempo between them.
            let pr = bounds.get(k + run + 1).map_or(pl, |&br| slope(t, b, br));
            // The left grid: its fitted period and phase, extended through the section. The
            // tolerance is 20 % of a beat (the tracker wobbles by up to a fifth of a beat around
            // bursts), capped at 70 ms so slow songs do not accept clearly shifted beats; a real
            // 5 % change in one bar of 120 BPM (26 to 105 ms off the grid) stays outside the share.
            let tol = (0.2 * pl).min(70.0);
            // Phase from the last two bars only: a long section's tempo wanders, so its mean phase
            // can be off by more than the tolerance at its end.
            let from = al.max(a.saturating_sub(2 * beats_per_bar as usize));
            let c = (from..=a).map(|i| t[i] - pl * (i as f64 - a as f64)).sum::<f64>() / (a - from + 1) as f64;
            let near = |x: f64, set: &[f64]| set.iter().any(|&y| (x - y).abs() <= tol);
            let inner = &t[a + 1..b];
            let end = if b == t.len() - 1 { t[b] + tol } else { t[b] - tol };
            let mut grid = Vec::new();
            let mut g = c + (((t[a] + tol - c) / pl).floor() + 1.0) * pl;
            while g < end {
                grid.push(g);
                g += pl;
            }
            let share = |hit: usize, all: usize| all > 0 && hit as f64 >= GLITCH_SHARE * all as f64;
            // Extra beats: at least half as many detected beats as grid beats again, and nearly all
            // grid beats among them. Missing beats: fewer detected beats than grid beats, nearly
            // all on the grid. A real change of a few percent has as many beats as the grid.
            let (n_in, n_grid) = (inner.len() as f64, grid.len() as f64);
            let extra = n_in >= 1.5 * n_grid && share(grid.iter().filter(|&&g| near(g, &t[a + 1..=b])).count(), grid.len());
            let missing = n_in <= 0.67 * n_grid && share(inner.iter().filter(|&&x| near(x, &grid)).count(), inner.len());
            if extra || missing || irregular {
                let period = if agrees { pl } else { (pl + pr) / 2.0 };
                if regrid(t, a, b, period, if agrees { Spacing::Start } else { Spacing::Even }) {
                    return true;
                }
            }
        }
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
enum Spacing {
    /// Exactly one period apart, counted from the section's first beat.
    Start,
    /// Exactly one period apart, counted back from the section's last beat.
    End,
    /// Evenly spaced, as close to one period as the span allows.
    Even,
}

/// Replace the beats strictly inside a..b by ones one `period` apart, so the section carries that
/// tempo (with `Start` / `End` the left-over gap at the other end is one odd interval, which the
/// segmentation absorbs into the neighbour). Returns false when nothing changed.
fn regrid(t: &mut Vec<f64>, a: usize, b: usize, period: f64, spacing: Spacing) -> bool {
    let n = (((t[b] - t[a]) / period).round() as usize).max(1);
    let (ta, tb) = (t[a], t[b]);
    let mut fill: Vec<f64> = (1..n)
        .map(|i| match spacing {
            Spacing::Start => ta + period * i as f64,
            Spacing::End => tb - period * (n - i) as f64,
            Spacing::Even => ta + (tb - ta) * i as f64 / n as f64,
        })
        .collect();
    fill.retain(|&x| x > ta + 1.0 && x < tb - 1.0);
    let same = fill.len() == b - a - 1 && fill.iter().zip(&t[a + 1..b]).all(|(x, y)| (x - y).abs() < 0.5);
    t.splice(a + 1..b, fill);
    !same
}

/// Move each change back over the beats before it that already sit on the right section's grid
/// (within 2 sigma), keeping at least one interval for the left section. The segmentation can
/// leave a wobbly lead-in beat or two on the wrong side of the change.
fn refine_changes(t: &[f64], bounds: &mut [usize], sigma: f64) {
    for k in 1..bounds.len() - 1 {
        let (b, e) = (bounds[k], bounds[k + 1]);
        let p = slope(t, b, e);
        let to = e.min(b + 8);
        let c = (b..=to).map(|i| t[i] - p * (i - b) as f64).sum::<f64>() / (to - b + 1) as f64;
        while bounds[k] > bounds[k - 1] + 1 {
            let i = bounds[k] - 1;
            if (t[i] - (c + p * (i as f64 - b as f64))).abs() > 2.0 * sigma {
                break;
            }
            bounds[k] = i;
        }
    }
}

/// Rule 2: a section at about 2x or 0.5x the tempo of its left neighbour (the right one for the
/// first section) gets its beats re-spaced at the neighbour's period (added for half time,
/// dropped for double time), so it carries the neighbour's tempo. Returns true when `t` changed.
fn fold_octave(t: &mut Vec<f64>, bounds: &[usize]) -> bool {
    let m = bounds.len() - 1;
    if m < 2 {
        return false;
    }
    for k in 0..m {
        let nb = if k == 0 { 1 } else { k - 1 };
        let (p, pn) = (slope(t, bounds[k], bounds[k + 1]), slope(t, bounds[nb], bounds[nb + 1]));
        // Only into a neighbour at least as long: the longer one is the song's tempo, not a glitch.
        let len = |i: usize| t[bounds[i + 1]] - t[bounds[i]];
        if len(k) > len(nb) {
            continue;
        }
        let octave = ratio_diff(p, 2.0 * pn) <= OCTAVE_TOL || ratio_diff(p, pn / 2.0) <= OCTAVE_TOL;
        if octave && regrid(t, bounds[k], bounds[k + 1], pn, if k == 0 { Spacing::End } else { Spacing::Start }) {
            return true;
        }
    }
    false
}

/// Beats and downbeats in seconds to a tempo map and its sections.
pub(crate) fn detect(beats_s: &[f64], downbeats_s: &[f64], _beats_per_bar: u32) -> Result<Detected, String> {
    let raw = clean(beats_s);
    if raw.len() < MIN_BEATS {
        return Err(NO_BEAT_ERROR.to_string());
    }
    let mut t = repair(&raw);
    if t.len() < MIN_BEATS {
        return Err(NO_BEAT_ERROR.to_string());
    }
    let down = clean(downbeats_s);
    let ibi: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    let dd: Vec<f64> = ibi.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let sigma = (1.4826 * median(&dd) / 2f64.sqrt()).max(5.0);

    // Apply the glitch and octave rules one change at a time, re-segmenting after each, until
    // nothing changes (bounded).
    let bpb = DEFAULT_BEATS_PER_BAR as f64;
    let mut round = 0;
    let mut bounds = loop {
        let mut bounds = segment(&t, sigma);
        merge_close(&t, &mut bounds);
        round += 1;
        if round > MAX_RULE_ROUNDS || !(absorb_glitch(&mut t, &bounds, sigma, bpb) || fold_octave(&mut t, &bounds)) {
            break bounds;
        }
    };

    refine_changes(&t, &mut bounds, sigma);

    // Quantise, then merge neighbours with the same rounded BPM.
    let quant = |bounds: &[usize]| -> Vec<(f64, f64)> {
        (0..bounds.len() - 1).map(|k| quantise(slope(&t, bounds[k], bounds[k + 1]))).collect()
    };
    let mut q = quant(&bounds);
    while let Some(k) = (0..q.len().saturating_sub(1)).find(|&k| q[k].0 == q[k + 1].0) {
        bounds.remove(k + 1);
        q = quant(&bounds);
    }

    // Anchor: the first downbeat in section 0 (nearest beat), else its first beat.
    let nearest = |x: f64| -> usize {
        let p = t.partition_point(|&b| b < x);
        if p == 0 {
            0
        } else if p == t.len() || x - t[p - 1] <= t[p] - x {
            p - 1
        } else {
            p
        }
    };
    let anchor_beat = down.iter().map(|&x| nearest(x)).find(|&i| i < bounds[1]).unwrap_or(0);
    let (bpm0, f0) = q[0];
    let anchor = round_ms(intercept(&t, bounds[0], bounds[1], anchor_beat, bpm0, f0));
    let mut segments = vec![Segment { start_ms: anchor, bpm: bpm0, detected_ms: None, on_beat: false }];
    for k in 1..q.len() {
        let (bpm, f) = q[k];
        let tk = intercept(&t, bounds[k], bounds[k + 1], bounds[k], bpm, f);
        let left = segments.last().unwrap();
        let fine = fine_ms(left.bpm);
        let start = left.start_ms + ((tk - left.start_ms) / fine).round() * fine;
        if start - left.start_ms < 60000.0 / left.bpm - 1e-6 || bpm == left.bpm {
            continue;
        }
        segments.push(Segment { start_ms: start, bpm, detected_ms: Some(start), on_beat: false });
    }
    let last = *t.last().unwrap();
    let sections = segments
        .iter()
        .enumerate()
        .map(|(k, s)| SectionInfo {
            start_ms: s.start_ms,
            end_ms: segments.get(k + 1).map_or(last.max(s.start_ms), |n| n.start_ms),
            bpm: s.bpm,
        })
        .collect();
    Ok(Detected {
        map: TempoMap { beats_per_bar: DEFAULT_BEATS_PER_BAR, segments },
        sections,
        sigma_ms: sigma,
        bar_beats: bar_length(&t, &down),
    })
}

#[cfg(test)]
#[path = "tempo_detect_tests.rs"]
mod tests;
