//! Play/pause/stop and the 0.3 s resume rule (plan 2.4). Time is passed in; the position
//! lives in the player, not here.

#![allow(dead_code)] // used by the editor session (task 7) and the tests

use std::time::{Duration, Instant};

/// Silence after the last position change before playback resumes.
pub const RESUME_DELAY: Duration = Duration::from_millis(300);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Transport {
    playing: bool,
    resume_at: Option<Instant>,
}

impl Transport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts or resumes; a pending resume is dropped, so the sound starts at once.
    pub fn play(&mut self) {
        self.playing = true;
        self.resume_at = None;
    }

    /// Pauses and cancels a pending resume.
    pub fn pause(&mut self) {
        self.playing = false;
        self.resume_at = None;
    }

    /// Stops and clears everything.
    pub fn stop(&mut self) {
        self.pause();
    }

    /// The track reached its end: same as stop.
    pub fn ended(&mut self) {
        self.stop();
    }

    /// A position change. While playing (or resume pending) it silences the output and
    /// moves the resume time to `now + RESUME_DELAY`; while paused it changes nothing.
    pub fn seek(&mut self, now: Instant) {
        if self.playing {
            self.resume_at = Some(now + RESUME_DELAY);
        }
    }

    /// Applies a due resume. Returns true when the state changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.resume_at {
            Some(at) if now >= at => {
                self.resume_at = None;
                true
            }
            _ => false,
        }
    }

    /// Whether sound should come out.
    pub fn audible(&self) -> bool {
        self.playing && self.resume_at.is_none()
    }

    /// Playing or resume pending (the button stays "Pause" during a scrub).
    pub fn shows_playing(&self) -> bool {
        self.playing
    }

    pub fn resume_pending(&self) -> bool {
        self.resume_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn play_is_audible() {
        let mut t = Transport::new();
        assert!(!t.audible() && !t.shows_playing());
        t.play();
        assert!(t.audible() && t.shows_playing());
    }

    #[test]
    fn pause_is_silent() {
        let mut t = Transport::new();
        t.play();
        t.pause();
        assert!(!t.audible() && !t.shows_playing());
    }

    #[test]
    fn seek_while_playing_resumes_after_300ms() {
        let t0 = Instant::now();
        let mut t = Transport::new();
        t.play();
        t.seek(t0);
        assert!(!t.audible() && t.shows_playing() && t.resume_pending());
        assert!(!t.tick(t0 + ms(299)));
        assert!(!t.audible());
        assert!(t.tick(t0 + ms(300)));
        assert!(t.audible() && !t.resume_pending());
        assert!(!t.tick(t0 + ms(400)));
    }

    #[test]
    fn second_seek_moves_resume() {
        let t0 = Instant::now();
        let mut t = Transport::new();
        t.play();
        t.seek(t0);
        t.seek(t0 + ms(200));
        t.tick(t0 + ms(499));
        assert!(!t.audible());
        t.tick(t0 + ms(500));
        assert!(t.audible());
    }

    #[test]
    fn seek_while_paused_does_nothing() {
        let t0 = Instant::now();
        let mut t = Transport::new();
        t.seek(t0);
        assert!(!t.shows_playing() && !t.resume_pending());
        t.tick(t0 + ms(1000));
        assert!(!t.audible());
        t.play();
        t.pause();
        t.seek(t0);
        t.tick(t0 + ms(1000));
        assert!(!t.audible() && !t.shows_playing());
    }

    #[test]
    fn pause_in_window_cancels_resume() {
        let t0 = Instant::now();
        let mut t = Transport::new();
        t.play();
        t.seek(t0);
        t.pause();
        assert!(!t.tick(t0 + ms(300)));
        assert!(!t.audible() && !t.shows_playing());
    }

    #[test]
    fn play_in_window_is_audible_at_once() {
        let t0 = Instant::now();
        let mut t = Transport::new();
        t.play();
        t.seek(t0);
        t.play();
        assert!(t.audible());
    }

    #[test]
    fn stop_and_ended_clear_everything() {
        let t0 = Instant::now();
        let mut t = Transport::new();
        t.play();
        t.seek(t0);
        t.stop();
        assert_eq!(t, Transport::new());
        t.play();
        t.ended();
        assert_eq!(t, Transport::new());
    }
}
