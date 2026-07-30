//! Input pipeline (Architecture spec §4.4, Hand Gestures spec).
//!
//! Fuses mouse/keyboard, hand gestures, and voice into one source-tagged
//! event stream. M2 wired the mouse/keyboard leg; M3 adds the hand pipeline
//! (`hands`): landmark ingestion, One-Euro filtering, pose classification.

pub mod hands;

use pmos_abi::{InputSource, KernelEvent};

pub struct InputPipeline {
    /// Events produced this frame, drained by the dispatcher.
    frame_events: Vec<KernelEvent>,
    /// Which source last drove the pointer — the shell styles the cursor
    /// with this (UI spec §3.1: "one pointer, many sources").
    pub active_source: InputSource,
    /// Hand tracking and gesture recognition state.
    pub hands: hands::HandsState,
}

impl InputPipeline {
    pub fn new() -> Self {
        Self {
            frame_events: Vec::new(),
            active_source: InputSource::Mouse,
            hands: hands::HandsState::new(),
        }
    }

    pub fn pointer_moved(&mut self, pos: [f32; 2], source: InputSource) {
        self.active_source = source;
        self.frame_events
            .push(KernelEvent::PointerMove { pos, source });
    }

    pub fn drain(&mut self) -> Vec<KernelEvent> {
        std::mem::take(&mut self.frame_events)
    }
}

impl Default for InputPipeline {
    fn default() -> Self {
        Self::new()
    }
}
