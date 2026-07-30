//! Pose → pointer-intent fusion (Hand Gestures spec §3, §5).
//!
//! Turns debounced hand poses into the pointer intents the platform feeds to
//! the UI toolkit: pinch = primary press/release, middle-pinch = secondary,
//! two-finger = scroll (pointer parks), grab = whole-hand drag (window move
//! or camera orbit — the platform decides by what's under the pointer).
//! Held buttons are ALWAYS released on tracking loss — a vanished hand must
//! never leave a button stuck down (spec §7).

use pmos_abi::HandPose;

/// One frame's worth of pointer intent, consumed by the platform glue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandIntent {
    Move([f32; 2]),
    Press { pos: [f32; 2], secondary: bool },
    Release { pos: [f32; 2], secondary: bool },
    Scroll { pos: [f32; 2], dy: f32 },
    GrabStart([f32; 2]),
    GrabMove([f32; 2]),
    GrabEnd([f32; 2]),
}

#[derive(Default)]
pub struct Fusion {
    prev_pose: Option<HandPose>,
    prev_cursor: Option<[f32; 2]>,
    primary_down: bool,
    secondary_down: bool,
    grabbing: bool,
    intents: Vec<HandIntent>,
}

impl Fusion {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one recognizer frame (post-debounce). `cursor` is in UI points.
    pub fn step(&mut self, pose: HandPose, cursor: [f32; 2], tracking: bool) {
        if !tracking {
            self.lost(cursor);
            return;
        }
        let entered = self.prev_pose != Some(pose);

        // Leaving a stateful pose releases whatever it held.
        if entered {
            if let Some(prev) = self.prev_pose {
                self.exit_pose(prev, cursor);
            }
        }

        match pose {
            HandPose::Pinch => {
                if entered && !self.primary_down {
                    self.intents.push(HandIntent::Move(cursor));
                    self.intents.push(HandIntent::Press {
                        pos: cursor,
                        secondary: false,
                    });
                    self.primary_down = true;
                } else {
                    self.intents.push(HandIntent::Move(cursor));
                }
            }
            HandPose::MiddlePinch => {
                if entered && !self.secondary_down {
                    self.intents.push(HandIntent::Move(cursor));
                    self.intents.push(HandIntent::Press {
                        pos: cursor,
                        secondary: true,
                    });
                    self.secondary_down = true;
                } else {
                    self.intents.push(HandIntent::Move(cursor));
                }
            }
            HandPose::TwoFinger => {
                // Scroll: the pointer parks; vertical hand motion becomes
                // wheel deltas (trackpad muscle memory, spec G7).
                if let Some(prev) = self.prev_cursor {
                    let dy = cursor[1] - prev[1];
                    if dy.abs() > 0.01 {
                        self.intents.push(HandIntent::Scroll { pos: prev, dy });
                    }
                }
            }
            HandPose::Grab => {
                if entered && !self.grabbing {
                    self.intents.push(HandIntent::GrabStart(cursor));
                    self.grabbing = true;
                } else {
                    self.intents.push(HandIntent::GrabMove(cursor));
                }
            }
            // Pointer-neutral poses just move the cursor.
            _ => self.intents.push(HandIntent::Move(cursor)),
        }

        self.prev_pose = Some(pose);
        // While scrolling the pointer stays parked so deltas accumulate
        // relative to the previous frame's hand position.
        if pose != HandPose::TwoFinger {
            self.prev_cursor = Some(cursor);
        } else if self.prev_cursor.is_none() {
            self.prev_cursor = Some(cursor);
        } else {
            // Track hand motion for the next delta without moving the pointer.
            let parked = self.prev_cursor.unwrap();
            self.prev_cursor = Some([parked[0], cursor[1]]);
        }
    }

    fn exit_pose(&mut self, prev: HandPose, cursor: [f32; 2]) {
        match prev {
            HandPose::Pinch if self.primary_down => {
                self.intents.push(HandIntent::Release {
                    pos: cursor,
                    secondary: false,
                });
                self.primary_down = false;
            }
            HandPose::MiddlePinch if self.secondary_down => {
                self.intents.push(HandIntent::Release {
                    pos: cursor,
                    secondary: true,
                });
                self.secondary_down = false;
            }
            HandPose::Grab if self.grabbing => {
                self.intents.push(HandIntent::GrabEnd(cursor));
                self.grabbing = false;
            }
            _ => {}
        }
    }

    /// Tracking lost: release everything held, freeze in place.
    pub fn lost(&mut self, cursor: [f32; 2]) {
        if self.primary_down {
            self.intents.push(HandIntent::Release {
                pos: cursor,
                secondary: false,
            });
            self.primary_down = false;
        }
        if self.secondary_down {
            self.intents.push(HandIntent::Release {
                pos: cursor,
                secondary: true,
            });
            self.secondary_down = false;
        }
        if self.grabbing {
            self.intents.push(HandIntent::GrabEnd(cursor));
            self.grabbing = false;
        }
        self.prev_pose = None;
        self.prev_cursor = None;
    }

    pub fn take_intents(&mut self) -> Vec<HandIntent> {
        std::mem::take(&mut self.intents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poses(seq: &[(HandPose, [f32; 2])]) -> Vec<HandIntent> {
        let mut f = Fusion::new();
        for &(p, c) in seq {
            f.step(p, c, true);
        }
        f.take_intents()
    }

    #[test]
    fn pinch_produces_press_then_release() {
        let out = poses(&[
            (HandPose::Point, [10.0, 10.0]),
            (HandPose::Pinch, [10.0, 10.0]),
            (HandPose::Pinch, [20.0, 12.0]),
            (HandPose::Point, [20.0, 12.0]),
        ]);
        assert!(out.contains(&HandIntent::Press {
            pos: [10.0, 10.0],
            secondary: false
        }));
        assert!(out.contains(&HandIntent::Release {
            pos: [20.0, 12.0],
            secondary: false
        }));
        // Press must come before release.
        let press = out
            .iter()
            .position(|i| matches!(i, HandIntent::Press { .. }))
            .unwrap();
        let release = out
            .iter()
            .position(|i| matches!(i, HandIntent::Release { .. }))
            .unwrap();
        assert!(press < release);
    }

    #[test]
    fn tracking_loss_releases_held_pinch() {
        let mut f = Fusion::new();
        f.step(HandPose::Pinch, [5.0, 5.0], true);
        f.step(HandPose::Pinch, [6.0, 5.0], false); // hand vanished
        let out = f.take_intents();
        assert!(out.iter().any(|i| matches!(
            i,
            HandIntent::Release {
                secondary: false,
                ..
            }
        )));
    }

    #[test]
    fn grab_lifecycle() {
        let out = poses(&[
            (HandPose::Grab, [1.0, 1.0]),
            (HandPose::Grab, [2.0, 2.0]),
            (HandPose::Rest, [3.0, 3.0]),
        ]);
        assert_eq!(out[0], HandIntent::GrabStart([1.0, 1.0]));
        assert!(out.contains(&HandIntent::GrabMove([2.0, 2.0])));
        assert!(out.contains(&HandIntent::GrabEnd([3.0, 3.0])));
    }

    #[test]
    fn two_finger_scrolls_without_moving() {
        let out = poses(&[
            (HandPose::Point, [50.0, 50.0]),
            (HandPose::TwoFinger, [50.0, 50.0]),
            (HandPose::TwoFinger, [50.0, 60.0]),
        ]);
        assert!(out
            .iter()
            .any(|i| matches!(i, HandIntent::Scroll { dy, .. } if (dy - 10.0).abs() < 0.5)));
        // No Move intents while scrolling.
        assert!(!out[1..].iter().any(|i| matches!(i, HandIntent::Move(_))));
    }
}
