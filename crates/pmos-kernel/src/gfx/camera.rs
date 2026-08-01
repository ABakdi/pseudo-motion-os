//! The stage orbit camera (UI spec §1: clamped zoom, always resettable).

use glam::{Mat4, Vec3};

pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: Vec3,
}

const DEFAULT: OrbitCamera = OrbitCamera {
    yaw: 0.0,
    pitch: 0.32,
    dist: 13.0,
    target: Vec3::new(0.0, 1.2, 0.0),
};
const PITCH_RANGE: (f32, f32) = (0.06, 1.35);
/// Zoom clamp — one of the guarantees that the galaxy stays unreachable.
const DIST_RANGE: (f32, f32) = (5.0, 26.0);

impl OrbitCamera {
    pub fn new() -> Self {
        DEFAULT
    }

    pub fn reset(&mut self) {
        *self = DEFAULT;
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.005;
        self.pitch = (self.pitch + dy * 0.005).clamp(PITCH_RANGE.0, PITCH_RANGE.1);
    }

    pub fn zoom(&mut self, delta: f32) {
        self.dist = (self.dist * (1.0 - delta * 0.1)).clamp(DIST_RANGE.0, DIST_RANGE.1);
    }

    /// Pan the orbit target in the view plane ("grab the world" feel:
    /// the scene follows the drag). Clamped so the stage stays in reach.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let scale = self.dist * 0.0016;
        self.target -= right * (dx * scale);
        self.target.y = (self.target.y + dy * scale).clamp(0.2, 8.0);
        self.target.x = self.target.x.clamp(-10.0, 10.0);
        self.target.z = self.target.z.clamp(-10.0, 10.0);
    }

    pub fn eye(&self) -> Vec3 {
        self.target
            + self.dist
                * Vec3::new(
                    self.pitch.cos() * self.yaw.sin(),
                    self.pitch.sin(),
                    self.pitch.cos() * self.yaw.cos(),
                )
    }

    fn proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(60f32.to_radians(), aspect.max(0.1), 0.1, 200.0)
    }

    /// Full view-projection — used by the floor and by icon projection.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    /// Rotation-only inverse view-projection for the sky shader. Translation
    /// is deliberately absent: the galaxy responds to where you *look*, never
    /// to where you *are* — which is what makes it unreachable (UI spec §1).
    pub fn inv_rot_proj(&self, aspect: f32) -> Mat4 {
        let rot_view = Mat4::look_at_rh(Vec3::ZERO, self.target - self.eye(), Vec3::Y);
        (self.proj(aspect) * rot_view).inverse()
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self::new()
    }
}
