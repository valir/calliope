//! Peaks of the current stem mix and the restartable 0.3 s debounce (plan 2.7). Pure: no threads
//! and no clock of its own (the caller passes `Instant`s).

use crate::mixer;
use crate::stem_audio::StemPcm;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const CHUNK_FRAMES: usize = 4096;
pub const BUCKETS: usize = 2048;

/// Max |sample| of the hard-clipped mix per bucket, using the bucket rule of
/// `player_source::peaks`. `gains` pairs with `stems`. Cancel is checked between chunks and gives
/// `None`. All gains 0 gives zeros at once.
pub fn mix_peaks(
    stems: &[StemPcm],
    gains: &[f32],
    total_frames: u64,
    buckets: usize,
    cancel: &AtomicBool,
) -> Option<Vec<f32>> {
    let mut out = vec![0f32; buckets];
    if buckets == 0 || total_frames == 0 || gains.iter().all(|&g| g == 0.0) {
        return Some(out);
    }
    let per = ((total_frames / buckets as u64).max(1)) as usize;
    let active: Vec<usize> = (0..stems.len().min(gains.len())).filter(|&k| gains[k] != 0.0).collect();
    let active_gains: Vec<f32> = active.iter().map(|&k| gains[k]).collect();
    let mut scratch: Vec<Vec<f32>> = active.iter().map(|_| vec![0f32; CHUNK_FRAMES * 2]).collect();
    let mut mixed = vec![0f32; CHUNK_FRAMES * 2];
    let mut done: u64 = 0;
    while done < total_frames {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let n = (total_frames - done).min(CHUNK_FRAMES as u64) as usize;
        for (s, &k) in scratch.iter_mut().zip(&active) {
            mixer::to_stereo_f32(&stems[k], done, n, s);
        }
        let inputs: Vec<&[f32]> = scratch.iter().map(|s| &s[..n * 2]).collect();
        mixer::mix(&inputs, &active_gains, &mut mixed[..n * 2]);
        for i in 0..n {
            let b = (((done + i as u64) / per as u64) as usize).min(buckets - 1);
            let v = mixed[2 * i].abs().max(mixed[2 * i + 1].abs());
            if v > out[b] {
                out[b] = v;
            }
        }
        done += n as u64;
    }
    Some(out)
}

/// Restartable deadline: a change restarts the wait, and the deadline fires once.
#[derive(Debug, Default)]
pub struct Debounce {
    due: Option<Instant>,
}

impl Debounce {
    pub const DELAY: Duration = Duration::from_millis(300);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn touch(&mut self, now: Instant) {
        self.due = Some(now + Self::DELAY);
    }

    pub fn is_pending(&self) -> bool {
        self.due.is_some()
    }

    /// True once when `now` has reached the deadline; clears it.
    pub fn take_due(&mut self, now: Instant) -> bool {
        match self.due {
            Some(d) if now >= d => {
                self.due = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "mix_waveform_tests.rs"]
mod tests;
