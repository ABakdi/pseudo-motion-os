//! The CSL sign engine (Computer Sign Language spec §3–§4): per-sign state
//! machines over the debounced pose stream. v1 implements RECORD.
//!
//! RECORD is a still ✋ open-palm HOLD (1.0 s) — "raise your hand to speak".
//! The original ✋→✊ squeeze collided head-on with ✊ = grab (user-reported:
//! opening the hand before grabbing kept toggling voice), so the fist now
//! belongs exclusively to grabbing and RECORD lives on the palm alone.
//!
//! Midas-touch guards (spec §3): the palm must be held AND still (drift
//! aborts — a palm waving mid-conversation is gesticulation, not a sign),
//! a fired sign has a refractory period and must be released (pose must
//! leave OpenPalm) before it can re-arm, and tracking loss aborts silently.

use pmos_abi::{CslSign, HandPose};

/// The palm must be held this long to fire RECORD.
const PALM_HOLD_SECS: f64 = 1.0;
/// Cursor drift beyond this (egui points) re-arms the hold — moving palms
/// are pointing/gesticulating, not signing.
const MAX_DRIFT_PT: f32 = 70.0;
/// No sign can fire again within this window of a completed one.
const REFRACTORY_SECS: f64 = 1.0;
/// COMMAND: ☝ Point held still this long marks the next utterance (spec §4).
/// (Chin-anchored once the face mesh provides the location parameter.)
const POINT_HOLD_SECS: f64 = 0.8;

#[derive(Default)]
pub struct SignEngine {
    /// When the still palm hold began (None = not armed).
    palm_since: Option<f64>,
    /// Cursor position at arm time (drift reference).
    anchor: Option<[f32; 2]>,
    /// Set after firing; the palm must drop before RECORD can re-arm.
    fired: bool,
    refractory_until: f64,
    // COMMAND (☝ still hold) — same guard structure as RECORD.
    point_since: Option<f64>,
    point_anchor: Option<[f32; 2]>,
    point_fired: bool,
}

impl SignEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.palm_since = None;
        self.anchor = None;
    }

    fn reset_point(&mut self) {
        self.point_since = None;
        self.point_anchor = None;
    }

    /// Feed one debounced pose frame (with the stabilized cursor position);
    /// returns a completed sign at most once per gesture.
    pub fn step(
        &mut self,
        pose: HandPose,
        pos: Option<[f32; 2]>,
        tracking: bool,
        now: f64,
    ) -> Option<CslSign> {
        if !tracking {
            self.reset();
            self.reset_point();
            self.fired = false;
            self.point_fired = false;
            return None;
        }
        // COMMAND: a still ☝ Point hold. Runs beside RECORD — the shell only
        // acts on it while voice capture is live, which keeps ordinary
        // pointing (which moves) from ever mattering.
        if pose == HandPose::Point {
            self.reset();
            self.fired = false;
            if self.point_fired || now < self.refractory_until {
                return None;
            }
            if let (Some(anchor), Some(p)) = (self.point_anchor, pos) {
                let drift = (p[0] - anchor[0]).abs() + (p[1] - anchor[1]).abs();
                if drift > MAX_DRIFT_PT {
                    self.point_since = Some(now);
                    self.point_anchor = pos;
                    return None;
                }
            }
            let since = *self.point_since.get_or_insert(now);
            if self.point_anchor.is_none() {
                self.point_anchor = pos;
            }
            if now - since >= POINT_HOLD_SECS {
                self.reset_point();
                self.point_fired = true;
                self.refractory_until = now + REFRACTORY_SECS;
                return Some(CslSign::Command);
            }
            return None;
        }
        self.reset_point();
        self.point_fired = false;
        if pose != HandPose::OpenPalm {
            // Palm dropped: release the fired latch, abort any partial hold.
            self.reset();
            self.fired = false;
            return None;
        }
        if self.fired || now < self.refractory_until {
            return None;
        }
        // Drift check: a moving palm is not a sign.
        if let (Some(anchor), Some(p)) = (self.anchor, pos) {
            let drift = (p[0] - anchor[0]).abs() + (p[1] - anchor[1]).abs();
            if drift > MAX_DRIFT_PT {
                self.palm_since = Some(now);
                self.anchor = pos;
                return None;
            }
        }
        let since = *self.palm_since.get_or_insert(now);
        if self.anchor.is_none() {
            self.anchor = pos;
        }
        if now - since >= PALM_HOLD_SECS {
            self.reset();
            self.fired = true;
            self.refractory_until = now + REFRACTORY_SECS;
            return Some(CslSign::Record);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(
        e: &mut SignEngine,
        pose: HandPose,
        pos: [f32; 2],
        from: f64,
        to: f64,
    ) -> Option<CslSign> {
        let mut t = from;
        while t <= to {
            if let Some(s) = e.step(pose, Some(pos), true, t) {
                return Some(s);
            }
            t += 1.0 / 30.0;
        }
        None
    }

    #[test]
    fn still_palm_hold_fires_record() {
        let mut e = SignEngine::new();
        assert_eq!(
            feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 0.0, 1.2),
            Some(CslSign::Record)
        );
    }

    #[test]
    fn short_or_moving_palm_does_not_fire() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 0.0, 0.5), None);
        // Big jump re-arms the hold — total time never accumulates.
        assert_eq!(feed(&mut e, HandPose::OpenPalm, [300.0, 100.0], 0.6, 1.4), None);
        assert_eq!(
            feed(&mut e, HandPose::OpenPalm, [300.0, 100.0], 1.5, 1.9),
            Some(CslSign::Record) // still again → fires 1s after the jump
        );
    }

    #[test]
    fn fist_never_fires_record() {
        // The grab pose is not part of RECORD anymore — no squeeze conflict.
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 0.0, 0.6), None);
        assert_eq!(feed(&mut e, HandPose::Grab, [100.0, 100.0], 0.65, 1.8), None);
    }

    #[test]
    fn must_release_palm_before_rearming() {
        let mut e = SignEngine::new();
        assert_eq!(
            feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 0.0, 1.2),
            Some(CslSign::Record)
        );
        // Palm kept raised: silent, even past the refractory window.
        assert_eq!(feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 1.3, 4.0), None);
        // Drop, raise again → fires again.
        assert_eq!(feed(&mut e, HandPose::Rest, [100.0, 100.0], 4.1, 4.2), None);
        assert_eq!(
            feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 4.3, 5.5),
            Some(CslSign::Record)
        );
    }

    #[test]
    fn still_point_hold_fires_command() {
        let mut e = SignEngine::new();
        assert_eq!(
            feed(&mut e, HandPose::Point, [200.0, 200.0], 0.0, 1.0),
            Some(CslSign::Command)
        );
        // Held pointing does not refire until the pose drops.
        assert_eq!(feed(&mut e, HandPose::Point, [200.0, 200.0], 1.1, 3.0), None);
    }

    #[test]
    fn tracking_loss_aborts() {
        let mut e = SignEngine::new();
        assert_eq!(feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 0.0, 0.8), None);
        assert_eq!(e.step(HandPose::OpenPalm, None, false, 0.85), None);
        assert_eq!(feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 0.9, 1.5), None);
        assert_eq!(
            feed(&mut e, HandPose::OpenPalm, [100.0, 100.0], 1.5, 2.1),
            Some(CslSign::Record)
        );
    }
}
