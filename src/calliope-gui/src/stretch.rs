//! Thin wrapper around Signalsmith Stretch (real-time pitch-preserving time-stretch) with the
//! alignment rule of the player speed design: after `align` the next output frame is song frame `target`.
//! Nothing here allocates after `Stretcher::new`.

use signalsmith_stretch::Stretch;

const CH: usize = 2;
/// Output frames processed at once while discarding in `align`.
const CHUNK: usize = 256;

pub(crate) struct Stretcher {
    inner: Stretch,
    pre_roll: usize,
    discard: Vec<f32>,
}

impl Stretcher {
    pub fn new(rate: u32) -> Self {
        let mut inner = Stretch::preset_default(CH as u32, rate);
        // The preset: block = rate * 0.12, interval = rate * 0.03.
        let pre_roll = (rate as f64 * 0.12) as usize + (rate as f64 * 0.03) as usize + 2;
        // The first seek grows an internal scratch vector; do it now.
        inner.seek(vec![0.0f32; pre_roll * CH], 1.0);
        inner.reset();
        Stretcher { inner, pre_roll, discard: vec![0.0; CHUNK * CH] }
    }

    pub fn input_latency(&self) -> usize {
        self.inner.input_latency()
    }

    pub fn output_latency(&self) -> usize {
        self.inner.output_latency()
    }

    /// Frames of pre-roll `seek` wants (at least block + interval).
    pub fn pre_roll_frames(&self) -> usize {
        self.pre_roll
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Interleaved stereo pre-roll; does not move the stream position.
    pub fn seek(&mut self, pre_roll: &[f32], speed: f64) {
        self.inner.seek(pre_roll, speed);
    }

    /// Interleaved stereo; `input` and `output` lengths set the speed (input/output).
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        self.inner.process(input, output);
    }

    /// Makes the next output frame song frame `target` at `speed`: reset, seek with the pre-roll
    /// before `target + input_latency()`, then feed from there and discard the first `output_latency()`
    /// output frames. `source(frame, buf)` fills stereo frames from song frame `frame` (zeros outside the
    /// track). `scratch` must hold at least `pre_roll_frames()` stereo frames and `2 * 256 * speed` frames.
    /// Returns the song frame the caller feeds next (`target + input_latency()` plus what the discard consumed).
    pub fn align(
        &mut self,
        source: impl Fn(u64, &mut [f32]),
        target: u64,
        speed: f64,
        scratch: &mut [f32],
    ) -> u64 {
        let l_in = self.input_latency() as u64;
        let l_out = self.output_latency();
        let feed = target + l_in;
        let pre = self.pre_roll;
        let cap = scratch.len() / CH;
        assert!(cap >= pre && cap as f64 >= (CHUNK as f64 * speed).ceil() + 1.0, "scratch too small");

        self.inner.reset();
        let buf = &mut scratch[..pre * CH];
        let missing = pre.saturating_sub(feed as usize);
        buf[..missing * CH].fill(0.0);
        if missing < pre {
            source(feed - (pre - missing) as u64, &mut buf[missing * CH..]);
        }
        self.inner.seek(&*buf, speed);

        let mut pos = feed;
        let mut acc = 0.0f64;
        let mut left = l_out;
        while left > 0 {
            let n = left.min(CHUNK);
            let exact = acc + n as f64 * speed;
            let m = exact.floor() as usize;
            acc = exact - m as f64;
            let input = &mut scratch[..m * CH];
            source(pos, input);
            self.inner.process(&*input, &mut self.discard[..n * CH]);
            pos += m as u64;
            left -= n;
        }
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn goertzel(x: &[f32], rate: f64, f: f64) -> f64 {
        let w = 2.0 * PI * f / rate;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s = v as f64 + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - c * s1 * s2
    }

    fn tone(frame: u64, rate: u32) -> f32 {
        (2.0 * PI * 440.0 * frame as f64 / rate as f64).sin() as f32 * 0.5
    }

    #[test]
    fn tone_keeps_pitch() {
        for rate in [22050u32, 44100] {
            for pct in [25u32, 50, 150, 200] {
                let s = pct as f64 / 100.0;
                let mut st = Stretcher::new(rate);
                let mut scratch = vec![0.0f32; st.pre_roll_frames() * CH * 2];
                let src = |f: u64, b: &mut [f32]| {
                    for (i, fr) in b.chunks_mut(2).enumerate() {
                        fr[0] = tone(f + i as u64, rate);
                        fr[1] = fr[0];
                    }
                };
                let mut pos = st.align(src, 5000, s, &mut scratch);
                let mut acc = 0.0;
                let mut mono = Vec::new();
                let mut out = vec![0.0f32; 256 * CH];
                let mut inp = vec![0.0f32; 512 * CH + 2];
                while mono.len() < rate as usize * 2 {
                    let e: f64 = acc + 256.0 * s;
                    let m = e.floor() as usize;
                    acc = e - m as f64;
                    src(pos, &mut inp[..m * CH]);
                    st.process(&inp[..m * CH], &mut out);
                    pos += m as u64;
                    mono.extend(out.chunks(2).map(|f| f[0]));
                }
                let seg = &mono[rate as usize..];
                let (mut best, mut bp) = (0.0, 0.0);
                let mut f = 400.0;
                while f <= 480.0 {
                    let p = goertzel(seg, rate as f64, f);
                    if p > best {
                        best = p;
                        bp = f;
                    }
                    f += 0.5;
                }
                let tol = if pct == 25 { 3.0 } else { 2.0 };
                assert!((bp - 440.0).abs() <= tol, "{rate} Hz {pct} %: {bp}");
            }
        }
    }

    #[test]
    fn output_length_at_85_percent() {
        let rate = 44100u32;
        let s = 0.85;
        let mut st = Stretcher::new(rate);
        let total_in = rate as usize * 10;
        let (mut fed, mut acc, mut produced) = (0usize, 0.0f64, 0usize);
        let mut out = vec![0.0f32; 256 * CH];
        let inp = vec![0.1f32; 512 * CH];
        // Output blocks of 256 frames, input follows from the speed; stop when the input is used up.
        loop {
            let e: f64 = acc + 256.0 * s;
            let m = e.floor() as usize;
            if fed + m > total_in {
                break;
            }
            acc = e - m as f64;
            st.process(&inp[..m * CH], &mut out);
            fed += m;
            produced += 256;
        }
        let secs = produced as f64 / rate as f64;
        assert!((secs - 11.76).abs() < 0.1176, "{secs}");
    }

    #[test]
    fn align_puts_bursts_where_expected() {
        let rate = 44100u32;
        let burst_len = rate as u64 / 100;
        let period = rate as u64 / 2;
        let src = |f: u64, b: &mut [f32]| {
            for (i, fr) in b.chunks_mut(2).enumerate() {
                let p = f + i as u64;
                let v = if p % period < burst_len {
                    (2.0 * PI * 1000.0 * p as f64 / rate as f64).sin() as f32 * 0.8
                } else {
                    0.0
                };
                fr[0] = v;
                fr[1] = v;
            }
        };
        let t = 33_333u64;
        for pct in [50u32, 85, 150] {
            let s = pct as f64 / 100.0;
            let mut st = Stretcher::new(rate);
            let mut scratch = vec![0.0f32; st.pre_roll_frames() * CH * 2];
            let mut pos = st.align(src, t, s, &mut scratch);
            let out_frames = rate as usize * 3;
            let mut mono = Vec::new();
            let (mut acc, mut out) = (0.0f64, vec![0.0f32; 256 * CH]);
            let mut inp = vec![0.0f32; 600 * CH];
            while mono.len() < out_frames {
                let e: f64 = acc + 256.0 * s;
                let m = e.floor() as usize;
                acc = e - m as f64;
                src(pos, &mut inp[..m * CH]);
                st.process(&inp[..m * CH], &mut out);
                pos += m as u64;
                mono.extend(out.chunks(2).map(|f| f[0]));
            }
            // Silent up to the first burst (the pre-roll is not played).
            let first_burst_out = (((33_333 / period + 1) * period) as f64 - t as f64) / s;
            let quiet = (first_burst_out as usize).saturating_sub(rate as usize / 20);
            let peak = mono[..quiet].iter().fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(peak < 0.02, "{pct} %: pre-roll audible, peak {peak}");
            // Each burst's 1 ms envelope peak is near the expected output frame.
            let win = (rate / 1000) as usize;
            for k in 0..4u64 {
                let mid = (33_333 / period + 1 + k) * period + burst_len / 2;
                let want = (mid as f64 - t as f64) / s;
                if want as usize + rate as usize / 50 > mono.len() {
                    break;
                }
                let (mut best, mut at) = (0.0f32, 0usize);
                let lo = (want as usize).saturating_sub(rate as usize / 25);
                let hi = (want as usize + rate as usize / 25).min(mono.len() - win);
                for i in lo..hi {
                    let e: f32 = mono[i..i + win].iter().map(|v| v * v).sum();
                    if e > best {
                        best = e;
                        at = i + win / 2;
                    }
                }
                let err = (at as f64 - want).abs() / rate as f64;
                assert!(err <= 0.010, "{pct} % burst {k}: off by {err} s");
            }
        }
    }
}
