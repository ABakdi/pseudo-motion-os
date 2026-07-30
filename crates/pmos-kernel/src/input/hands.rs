//! Hand landmark ingestion and gesture recognition (Hand Gestures spec §2–§3).
//!
//! Input: 21 MediaPipe landmarks per hand (normalized [0,1] camera space,
//! x,y,z interleaved), delivered by the gesture worker via the platform glue.
//! Output: a filtered cursor position, a debounced pose, and pinch progress —
//! rule-based on landmark topology, deliberately not a trained classifier
//! (deterministic, tunable, debuggable).

use pmos_abi::HandPose;

// ---------- One-Euro filter (jitter removal with low latency) ----------

struct LowPass {
    y: Option<f32>,
}

impl LowPass {
    fn apply(&mut self, x: f32, alpha: f32) -> f32 {
        let y = match self.y {
            Some(prev) => prev + alpha * (x - prev),
            None => x,
        };
        self.y = Some(y);
        y
    }
}

pub struct OneEuro {
    min_cutoff: f32,
    beta: f32,
    x: LowPass,
    dx: LowPass,
}

impl OneEuro {
    /// Defaults tuned for cursor control ("Balanced" preset, spec §6).
    pub fn cursor() -> Self {
        Self::preset(1)
    }

    /// 0 = Precise (fast, more jitter), 1 = Balanced, 2 = Smooth (laggy calm).
    pub fn preset(p: u8) -> Self {
        let (min_cutoff, beta) = match p {
            0 => (1.8, 0.03),
            2 => (0.5, 0.003),
            _ => (1.0, 0.007),
        };
        Self {
            min_cutoff,
            beta,
            x: LowPass { y: None },
            dx: LowPass { y: None },
        }
    }

    fn alpha(cutoff: f32, dt: f32) -> f32 {
        let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
        1.0 / (1.0 + tau / dt.max(1e-4))
    }

    pub fn filter(&mut self, x: f32, dt: f32) -> f32 {
        let prev = self.x.y.unwrap_or(x);
        let dx = (x - prev) / dt.max(1e-4);
        let edx = self.dx.apply(dx, Self::alpha(1.0, dt));
        let cutoff = self.min_cutoff + self.beta * edx.abs();
        self.x.apply(x, Self::alpha(cutoff, dt))
    }

    pub fn reset(&mut self) {
        self.x.y = None;
        self.dx.y = None;
    }
}

// ---------- tuning (defaults from Hand Gestures spec §6) ----------

/// Control box: the sub-region of camera space mapped to the full screen, so
/// small comfortable hand motions cover everything (spec §2).
const BOX_X: (f32, f32) = (0.22, 0.78);
const BOX_Y: (f32, f32) = (0.22, 0.78);
/// Pinch hysteresis (normalized by palm scale): enter below, exit above.
const PINCH_ENTER: f32 = 0.35;
const PINCH_EXIT: f32 = 0.55;
/// Frames a candidate pose must persist before becoming active.
const DEBOUNCE_FRAMES: u8 = 3;
const DEBOUNCE_FRAMES_PINCH: u8 = 2;
/// No landmarks for this long → tracking lost (cursor freezes, spec §7).
const LOST_AFTER_SECS: f64 = 0.5;

// ---------- landmark indices (MediaPipe hand model) ----------

const WRIST: usize = 0;
const THUMB_IP: usize = 3;
const THUMB_TIP: usize = 4;
const INDEX_PIP: usize = 6;
const INDEX_TIP: usize = 8;
const MIDDLE_MCP: usize = 9;
const MIDDLE_PIP: usize = 10;
const MIDDLE_TIP: usize = 12;
const RING_PIP: usize = 14;
const RING_TIP: usize = 16;
const PINKY_PIP: usize = 18;
const PINKY_TIP: usize = 20;

fn pt(lm: &[f32], i: usize) -> (f32, f32) {
    (lm[i * 3], lm[i * 3 + 1])
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

// ---------- recognizer ----------

pub struct HandsState {
    pub pose: HandPose,
    pub pinch: f32,
    /// Filtered cursor position in egui points.
    pub cursor: Option<[f32; 2]>,
    pub tracking: bool,
    pub hands: u8,
    pub camera_enabled: bool,
    pinch_latched: bool,
    candidate: HandPose,
    candidate_frames: u8,
    fx: OneEuro,
    fy: OneEuro,
    last_seen: f64,
    last_frame: f64,
    /// Runtime-tunable pinch thresholds (Hand Tracker app, ABI 1.2).
    pinch_enter: f32,
    pinch_exit: f32,
}

impl HandsState {
    pub fn new() -> Self {
        Self {
            pose: HandPose::Rest,
            pinch: 0.0,
            cursor: None,
            tracking: false,
            hands: 0,
            camera_enabled: false,
            pinch_latched: false,
            candidate: HandPose::Rest,
            candidate_frames: 0,
            fx: OneEuro::cursor(),
            fy: OneEuro::cursor(),
            last_seen: 0.0,
            last_frame: 0.0,
            pinch_enter: PINCH_ENTER,
            pinch_exit: PINCH_EXIT,
        }
    }

    /// Apply the recognizer-side fields of a tuning update (ABI 1.2).
    pub fn apply_tuning(&mut self, t: &pmos_abi::HandsTuning) {
        self.pinch_enter = t.pinch_enter.clamp(0.1, 0.5);
        self.pinch_exit = t.pinch_exit.clamp(self.pinch_enter + 0.05, 0.9);
        self.fx = OneEuro::preset(t.smoothing);
        self.fy = OneEuro::preset(t.smoothing);
    }

    /// Ingest one worker frame. `data` holds `hands * 63` floats.
    pub fn ingest(&mut self, data: &[f32], hands: u32, viewport: [f32; 2], now: f64) {
        self.hands = hands.min(2) as u8;
        if hands == 0 || data.len() < 63 {
            self.tick(now);
            return;
        }
        let dt = (now - self.last_frame).clamp(0.001, 0.2) as f32;
        self.last_frame = now;
        self.last_seen = now;
        self.tracking = true;

        let lm = &data[..63]; // primary hand
        let raw_pose = classify(lm, self.pinch_enter);

        // Debounce: a pose becomes active after N consecutive frames.
        if raw_pose == self.candidate {
            self.candidate_frames = self.candidate_frames.saturating_add(1);
        } else {
            self.candidate = raw_pose;
            self.candidate_frames = 1;
        }
        let needed = if matches!(raw_pose, HandPose::Pinch | HandPose::MiddlePinch) {
            DEBOUNCE_FRAMES_PINCH
        } else {
            DEBOUNCE_FRAMES
        };
        if self.candidate_frames >= needed {
            self.pose = self.candidate;
        }

        // Pinch progress with hysteresis (drives the tightening ring).
        let scale = dist(pt(lm, WRIST), pt(lm, MIDDLE_MCP)).max(1e-3);
        let pinch_d = dist(pt(lm, THUMB_TIP), pt(lm, INDEX_TIP)) / scale;
        self.pinch_latched = if self.pinch_latched {
            pinch_d < self.pinch_exit
        } else {
            pinch_d < self.pinch_enter
        };
        self.pinch = ((self.pinch_exit * 1.6 - pinch_d)
            / (self.pinch_exit * 1.6 - self.pinch_enter))
            .clamp(0.0, 1.0);

        // Cursor: index tip for precise poses, palm centroid otherwise.
        let anchor = match self.pose {
            HandPose::Point | HandPose::Pinch | HandPose::TwoFinger => pt(lm, INDEX_TIP),
            _ => {
                let ids = [0usize, 5, 9, 13, 17];
                let (sx, sy) = ids.iter().fold((0.0, 0.0), |acc, &i| {
                    (acc.0 + pt(lm, i).0, acc.1 + pt(lm, i).1)
                });
                (sx / 5.0, sy / 5.0)
            }
        };
        // Mirror x (webcam shows a mirror image), map through the control box.
        let nx = ((1.0 - anchor.0) - BOX_X.0) / (BOX_X.1 - BOX_X.0);
        let ny = (anchor.1 - BOX_Y.0) / (BOX_Y.1 - BOX_Y.0);
        let px = self.fx.filter(nx.clamp(0.0, 1.0) * viewport[0], dt);
        let py = self.fy.filter(ny.clamp(0.0, 1.0) * viewport[1], dt);
        self.cursor = Some([px, py]);
    }

    /// Time-based upkeep; call every frame regardless of worker delivery.
    pub fn tick(&mut self, now: f64) {
        if self.tracking && now - self.last_seen > LOST_AFTER_SECS {
            // Tracking lost: freeze the cursor, never jump (spec §7).
            self.tracking = false;
            self.pose = HandPose::Rest;
            self.pinch = 0.0;
            self.candidate_frames = 0;
            self.fx.reset();
            self.fy.reset();
        }
    }
}

impl Default for HandsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Rule-based static pose classification on landmark topology (spec §3).
fn classify(lm: &[f32], pinch_enter: f32) -> HandPose {
    let wrist = pt(lm, WRIST);
    let scale = dist(wrist, pt(lm, MIDDLE_MCP)).max(1e-3);

    let ext = |tip: usize, pip: usize, factor: f32| -> bool {
        dist(pt(lm, tip), wrist) > dist(pt(lm, pip), wrist) * factor
    };
    let thumb = ext(THUMB_TIP, THUMB_IP, 1.1);
    let index = ext(INDEX_TIP, INDEX_PIP, 1.15);
    let middle = ext(MIDDLE_TIP, MIDDLE_PIP, 1.15);
    let ring = ext(RING_TIP, RING_PIP, 1.15);
    let pinky = ext(PINKY_TIP, PINKY_PIP, 1.15);

    let pinch_d = dist(pt(lm, THUMB_TIP), pt(lm, INDEX_TIP)) / scale;
    let mid_pinch_d = dist(pt(lm, THUMB_TIP), pt(lm, MIDDLE_TIP)) / scale;

    // Priority: fingertip-touch poses beat finger-count poses.
    if pinch_d < pinch_enter {
        return HandPose::Pinch;
    }
    if mid_pinch_d < pinch_enter && !index {
        return HandPose::MiddlePinch;
    }
    match (thumb, index, middle, ring, pinky) {
        (_, true, true, true, true) => HandPose::OpenPalm,
        (true, false, false, false, true) => HandPose::CallSign,
        (_, true, true, false, false) => HandPose::TwoFinger,
        (_, true, false, false, false) => HandPose::Point,
        (true, false, false, false, false) => {
            // Thumb direction disambiguates up from down (y grows downward).
            if pt(lm, THUMB_TIP).1 < wrist.1 - 0.4 * scale {
                HandPose::ThumbsUp
            } else {
                HandPose::ThumbsDown
            }
        }
        (false, false, false, false, false) => HandPose::Grab,
        _ => HandPose::Rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a curled fist with realistic proportions (palm scale 0.1),
    /// then apply per-landmark offsets to pose it.
    fn hand(offsets: &[(usize, f32, f32)]) -> Vec<f32> {
        let mut lm = vec![0.0f32; 63];
        let base: &[(usize, f32, f32)] = &[
            (WRIST, 0.0, 0.0),
            (MIDDLE_MCP, 0.0, -0.10), // palm scale reference
            // curled fingers: tips slightly closer to the wrist than pips,
            // spread in x so no two fingertips touch
            (THUMB_IP, 0.10, -0.07),
            (THUMB_TIP, 0.09, -0.04),
            (INDEX_PIP, 0.02, -0.12),
            (INDEX_TIP, 0.02, -0.10),
            (MIDDLE_PIP, -0.01, -0.12),
            (MIDDLE_TIP, -0.01, -0.10),
            (RING_PIP, -0.04, -0.115),
            (RING_TIP, -0.04, -0.09),
            (PINKY_PIP, -0.07, -0.11),
            (PINKY_TIP, -0.07, -0.08),
        ];
        for i in 0..21 {
            lm[i * 3] = 0.5;
            lm[i * 3 + 1] = 0.5;
        }
        for &(i, dx, dy) in base.iter().chain(offsets) {
            lm[i * 3] = 0.5 + dx;
            lm[i * 3 + 1] = 0.5 + dy;
        }
        lm
    }

    #[test]
    fn classifies_point() {
        // Index far from wrist, its pip halfway, everything else curled.
        let lm = hand(&[
            (INDEX_TIP, 0.0, -0.3),
            (INDEX_PIP, 0.0, -0.15),
            (THUMB_TIP, 0.15, 0.0),
        ]);
        assert_eq!(classify(&lm, PINCH_ENTER), HandPose::Point);
    }

    #[test]
    fn classifies_pinch_by_fingertip_touch() {
        // Thumb and index tips together but away from the wrist.
        let lm = hand(&[
            (THUMB_TIP, 0.2, -0.2),
            (INDEX_TIP, 0.21, -0.2),
            (INDEX_PIP, 0.1, -0.1),
        ]);
        assert_eq!(classify(&lm, PINCH_ENTER), HandPose::Pinch);
    }

    #[test]
    fn classifies_grab_when_all_curled() {
        let lm = hand(&[]);
        assert_eq!(classify(&lm, PINCH_ENTER), HandPose::Grab);
    }

    #[test]
    fn one_euro_converges() {
        let mut f = OneEuro::cursor();
        let mut y = 0.0;
        for _ in 0..120 {
            y = f.filter(100.0, 1.0 / 60.0);
        }
        assert!((y - 100.0).abs() < 1.0);
    }
}
