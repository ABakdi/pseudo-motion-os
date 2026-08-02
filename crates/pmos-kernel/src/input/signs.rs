//! The CSL sign engine (Computer Sign Language spec §3–§4): per-sign state
//! machines over the debounced pose stream. v1 implements RECORD — the
//! ✋→✊ "squeeze" that toggles Voice Kit capture. COMMAND and CANCEL land
//! with their milestones (COMMAND needs the command stream, CANCEL needs
//! palm-z velocity).
//!
//! Midas-touch guards (spec §3): a sign needs its entry pose HELD (not just
//! classified for a frame), transitions have tight time windows, a fired
//! sign has a refractory period, and tracking loss aborts silently.

use pmos_abi::{CslSign, HandPose};

/// Entry pose must be held at least this long (spec §3 guard).
const PALM_HOLD_SECS: f64 = 0.3;
/// The fist must land within this long of the palm breaking.
const SQUEEZE_WINDOW_SECS: f64 = 0.8;
/// No sign can fire again within this window of a completed one.
const REFRACTORY_SECS: f64 = 0.5;

#[derive(Default)]
pub struct SignEngine {
    /// When the open palm began (None = not in a RECORD attempt).
    palm_since: Option<f64>,
    /// When the palm broke after a long-enough hold (squeeze in flight).
    palm_left_at: Option<f64>,
    refractory_until: f64,
}

impl SignEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.palm_since = None;
        self.palm_left_at = None;
    }

    /// Feed one debounced pose frame; returns a completed sign at most once
    /// per gesture.
    pub fn step(&mut self, pose: HandPose, tracking: bool, now: f64) -> Option<CslSign> {
        if !tracking || now < self.refractory_until {
            self.reset();
            return None;
        }
        match pose {
            HandPose::OpenPalm => {
                // (Re)arm: an open palm always restarts the hold timer chain.
                if self.palm_left_at.is_some() {
                    self.reset();
                }
                self.palm_since.get_or_insert(now);
            }
            HandPose::Grab => {
                if let Some(since) = self.palm_since {
                    let left = self.palm_left_at.unwrap_or(now);
                    let held = left - since;
                    let gap = now - left;
                    self.reset();
                    if held >= PALM_HOLD_SECS && gap <= SQUEEZE_WINDOW_SECS {
                        self.refractory_until = now + REFRACTORY_SECS;
                        return Some(CslSign::Record);
                    }
                }
            }
            // The pose stream often passes through Rest mid-squeeze (fingers
            // half-curled classify as neither palm nor fist). Tolerate it
            // inside the squeeze window; expire stale attempts.
            HandPose::Rest => {
                if let Some(since) = self.palm_since {
                    let left = *self.palm_left_at.get_or_insert(now);
                    if left - since < PALM_HOLD_SECS || now - left > SQUEEZE_WINDOW_SECS {
                        self.reset();
                    }
                }
            }
            // Any other deliberate pose is a different intent — abort.
            _ => self.reset(),
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(e: &mut SignEngine, pose: HandPose, from: f64, to: f64) -> Option<CslSign> {
        let mut t = from;
        while t <= to {
            if let Some(s) = e.step(pose, true, t) {
                return Some(s);
            }
            t += 1.0 / 30.0;
        }
        None
    }

    #[test]
    fn squeeze_fires_record() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, 0.0, 0.5), None);
        assert_eq!(feed(&mut e, HandPose::Rest, 0.53, 0.6), None);
        assert_eq!(
            feed(&mut e, HandPose::Grab, 0.63, 0.7),
            Some(CslSign::Record)
        );
    }

    #[test]
    fn short_palm_does_not_fire() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, 0.0, 0.1), None);
        assert_eq!(feed(&mut e, HandPose::Grab, 0.15, 0.4), None);
    }

    #[test]
    fn slow_squeeze_expires() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, 0.0, 0.5), None);
        assert_eq!(feed(&mut e, HandPose::Rest, 0.55, 1.6), None); // > window
        assert_eq!(feed(&mut e, HandPose::Grab, 1.65, 1.8), None);
    }

    #[test]
    fn other_pose_aborts_and_refractory_blocks() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, 0.0, 0.5), None);
        assert_eq!(feed(&mut e, HandPose::Point, 0.55, 0.6), None); // abort
        assert_eq!(feed(&mut e, HandPose::Grab, 0.65, 0.8), None);

        // A clean squeeze fires, then an immediate second one is suppressed.
        let mut e = SignEngine::new();
        assert!(feed(&mut e, HandPose::OpenPalm, 0.0, 0.5).is_none());
        assert_eq!(
            feed(&mut e, HandPose::Grab, 0.53, 0.6),
            Some(CslSign::Record)
        );
        assert_eq!(feed(&mut e, HandPose::OpenPalm, 0.65, 0.9), None);
    }

    #[test]
    fn tracking_loss_aborts() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, 0.0, 0.5), None);
        assert_eq!(e.step(HandPose::OpenPalm, false, 0.55), None); // lost
        assert_eq!(feed(&mut e, HandPose::Grab, 0.6, 0.7), None);
    }
}
