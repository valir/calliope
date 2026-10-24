//! Pure speed rules: the track speed in percent (steps, checks, the stored factor) and the
//! speed-training settings with the speed and progress of every pass of a loop run.

use serde::{Deserialize, Serialize};

pub const MIN_PCT: u32 = 25;
pub const MAX_PCT: u32 = 200;
pub const STEP_PCT: u32 = 5;
pub const SPEED_RANGE: &str = "The speed must be a whole number from 25 to 200 %";

const TRAINING_SPEEDS: &str = "Training speeds must be between 25 and 200 %";
const TRAINING_STEP: &str = "The step must be between 1 and 175 %";
const TRAINING_REPEATS: &str = "Repeats must be between 1 and 99";

/// Most presses honoured per call: 40 steps cross the whole 25..200 range.
const MAX_STEPS: u32 = 40;

/// `steps` presses of `-` (negative) or `+`: each moves to the next multiple of 5 in that direction; clamped.
pub fn step_pct(pct: u32, steps: i32) -> u32 {
    let mut p = pct.clamp(MIN_PCT, MAX_PCT);
    // the count comes from untrusted IPC
    for _ in 0..steps.unsigned_abs().min(MAX_STEPS) {
        p = if steps > 0 {
            (p / STEP_PCT + 1) * STEP_PCT
        } else {
            (p.div_ceil(STEP_PCT) - 1) * STEP_PCT
        };
        p = p.clamp(MIN_PCT, MAX_PCT);
    }
    p
}

/// A typed speed: Ok for 25..=200, else Err(SPEED_RANGE).
pub fn check_pct(pct: i64) -> Result<u32, String> {
    if (MIN_PCT as i64..=MAX_PCT as i64).contains(&pct) {
        Ok(pct as u32)
    } else {
        Err(SPEED_RANGE.to_string())
    }
}

/// The stored factor (`player.json` "speed") to percent: round(speed x 100), clamped; not finite is 100.
pub fn pct_of_factor(speed: f64) -> u32 {
    if !speed.is_finite() {
        return 100;
    }
    (speed * 100.0)
        .round()
        .clamp(MIN_PCT as f64, MAX_PCT as f64) as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Training {
    pub on: bool,
    pub start: u32,
    pub end: u32,
    pub step: u32,
    pub repeats: u32,
}

impl Default for Training {
    fn default() -> Self {
        Training {
            on: false,
            start: 70,
            end: 100,
            step: 5,
            repeats: 2,
        }
    }
}

/// What the Loops pane shows for a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub speed_pct: u32,
    pub repeat: u32,
    pub of: Option<u32>,
}

impl Training {
    /// start/end 25..=200, step 1..=175, repeats 1..=99.
    pub fn validate(&self) -> Result<(), String> {
        let speeds = MIN_PCT..=MAX_PCT;
        if !speeds.contains(&self.start) || !speeds.contains(&self.end) {
            return Err(TRAINING_SPEEDS.to_string());
        }
        if !(1..=175).contains(&self.step) {
            return Err(TRAINING_STEP.to_string());
        }
        if !(1..=99).contains(&self.repeats) {
            return Err(TRAINING_REPEATS.to_string());
        }
        Ok(())
    }

    /// Number of steps from start to end: 0 when equal, else ceil(|end - start| / step).
    pub fn steps(&self) -> u32 {
        let gap = self.start.abs_diff(self.end);
        let step = self.step.max(1);
        gap.div_ceil(step)
    }

    /// Speed of pass `pass` (0-based, counted from the loop Play).
    pub fn pass_speed(&self, pass: u32) -> u32 {
        let steps = self.steps();
        let i = (pass / self.repeats.max(1)).min(steps);
        if i == steps {
            self.end
        } else if self.end > self.start {
            self.start + i * self.step
        } else {
            self.start - i * self.step
        }
    }

    /// What the Loops pane shows for that pass.
    pub fn progress(&self, pass: u32) -> Progress {
        let repeats = self.repeats.max(1);
        let steps = self.steps();
        let speed_pct = self.pass_speed(pass);
        if pass / repeats < steps {
            Progress {
                speed_pct,
                repeat: pass % repeats + 1,
                of: Some(repeats),
            }
        } else {
            Progress {
                speed_pct,
                repeat: pass - steps * repeats + 1,
                of: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(start: u32, end: u32, step: u32, repeats: u32) -> Training {
        Training {
            on: true,
            start,
            end,
            step,
            repeats,
        }
    }

    #[test]
    fn step_pct_moves_to_multiples_of_five() {
        for (pct, steps, want) in [
            (100, -3, 85),
            (73, -1, 70),
            (73, 1, 75),
            (70, 1, 75),
            (70, -1, 65),
            (25, -1, 25),
            (27, -1, 25),
            (200, 1, 200),
            (198, 1, 200),
            (100, 0, 100),
            (300, -1, 195),
            (100, i32::MAX, 200),
            (100, i32::MIN, 25),
        ] {
            assert_eq!(step_pct(pct, steps), want, "({pct}, {steps})");
        }
    }

    #[test]
    fn check_pct_accepts_the_range_only() {
        assert_eq!(check_pct(25), Ok(25));
        assert_eq!(check_pct(200), Ok(200));
        for bad in [20, 24, 201, 250, -5] {
            assert_eq!(check_pct(bad), Err(SPEED_RANGE.to_string()), "{bad}");
        }
    }

    #[test]
    fn pct_of_factor_rounds_and_clamps() {
        for (f, want) in [
            (1.0, 100),
            (0.8, 80),
            (0.73, 73),
            (0.1, 25),
            (9.0, 200),
            (f64::NAN, 100),
        ] {
            assert_eq!(pct_of_factor(f), want, "{f}");
        }
    }

    #[test]
    fn default_and_validate() {
        assert_eq!(
            Training::default(),
            Training {
                on: false,
                start: 70,
                end: 100,
                step: 5,
                repeats: 2
            }
        );
        assert_eq!(Training::default().validate(), Ok(()));
        assert_eq!(t(60, 80, 10, 2).validate(), Ok(()));
        for bad in [t(20, 80, 10, 2), t(60, 201, 10, 2)] {
            assert_eq!(bad.validate(), Err(TRAINING_SPEEDS.to_string()));
        }
        for bad in [t(60, 80, 0, 2), t(60, 80, 176, 2)] {
            assert_eq!(bad.validate(), Err(TRAINING_STEP.to_string()));
        }
        for bad in [t(60, 80, 10, 0), t(60, 80, 10, 100)] {
            assert_eq!(bad.validate(), Err(TRAINING_REPEATS.to_string()));
        }
    }

    #[test]
    fn climbing_run() {
        let tr = t(60, 80, 10, 2);
        assert_eq!(tr.steps(), 2);
        let speeds: Vec<u32> = (0..=6).map(|p| tr.pass_speed(p)).collect();
        assert_eq!(speeds, [60, 60, 70, 70, 80, 80, 80]);
        let want = [
            (60, 1, Some(2)),
            (60, 2, Some(2)),
            (70, 1, Some(2)),
            (70, 2, Some(2)),
            (80, 1, None),
            (80, 2, None),
            (80, 3, None),
        ];
        for (p, (speed_pct, repeat, of)) in want.into_iter().enumerate() {
            assert_eq!(
                tr.progress(p as u32),
                Progress {
                    speed_pct,
                    repeat,
                    of
                },
                "pass {p}"
            );
        }
    }

    #[test]
    fn slowing_down_run() {
        let tr = t(100, 60, 15, 1);
        assert_eq!(tr.steps(), 3);
        let speeds: Vec<u32> = (0..=4).map(|p| tr.pass_speed(p)).collect();
        assert_eq!(speeds, [100, 85, 70, 60, 60]);
    }

    #[test]
    fn last_step_is_clamped_to_the_end() {
        let speeds: Vec<u32> = (0..=7).map(|p| t(60, 85, 10, 2).pass_speed(p)).collect();
        assert_eq!(speeds, [60, 60, 70, 70, 80, 80, 85, 85]);
    }

    #[test]
    fn start_equal_to_end_plays_at_that_speed() {
        let tr = t(80, 80, 5, 3);
        assert_eq!(tr.steps(), 0);
        assert_eq!(tr.pass_speed(0), 80);
        assert_eq!(
            tr.progress(0),
            Progress {
                speed_pct: 80,
                repeat: 1,
                of: None
            }
        );
        assert_eq!(
            tr.progress(4),
            Progress {
                speed_pct: 80,
                repeat: 5,
                of: None
            }
        );
    }
}
