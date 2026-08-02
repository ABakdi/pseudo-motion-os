//! Physics (Architecture spec §4.2): rapier3d rigid bodies on a fixed
//! 120 Hz timestep with render-rate decoupling. The stage props are real
//! dynamic bodies; grabbing attaches a kinematic spring (the body stays
//! dynamic so collisions keep resolving), and release inherits velocity —
//! which is what makes throwing feel free.

use glam::Vec3;
use rapier3d::prelude::*;

const DT: f32 = 1.0 / 120.0;
const MAX_STEPS_PER_FRAME: u32 = 6;

pub struct Prop {
    pub body: RigidBodyHandle,
    /// 0 = cube, 1 = sphere.
    pub shape: u8,
    pub half: f32,
    pub color: [f32; 3],
}

/// A note floating in the graph view (Notes System spec: notes-as-bodies).
pub struct NoteNode {
    pub body: RigidBodyHandle,
    pub title: String,
    pub path: String,
}

pub struct Physics {
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    pub bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    query: QueryPipeline,
    integration: IntegrationParameters,
    accumulator: f32,
    pub props: Vec<Prop>,
    grabbed: Option<(RigidBodyHandle, f32)>,
    /// Prop index under the pointer this frame (hover glow).
    pub hovered: Option<usize>,
    /// The FOCUSED object (pinch-tap select, CSL spec §5): outlined, named
    /// in the AI context, target of the non-dominant property controls.
    pub focused: Option<usize>,
    /// Notes-graph nodes (gravity-free bodies) + wikilink springs.
    pub notes: Vec<NoteNode>,
    pub note_links: Vec<(usize, usize)>,
}

impl Physics {
    pub fn new() -> Self {
        let bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();

        // The stage floor (matches the rendered grid plane at y = 0).
        colliders.insert(
            ColliderBuilder::cuboid(60.0, 0.5, 60.0)
                .translation(vector![0.0, -0.5, 0.0])
                .friction(0.8)
                .build(),
        );

        // The stage boots CLEAN (user decision 2026-08-01) — no scattered
        // demo props. `spawn_prop` stays: notes-as-bodies, conjured objects,
        // and the future stage_spawn tool all use it.
        Self {
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            bodies,
            colliders,
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            query: QueryPipeline::new(),
            integration: IntegrationParameters {
                dt: DT,
                ..Default::default()
            },
            accumulator: 0.0,
            props: Vec::new(),
            grabbed: None,
            hovered: None,
            focused: None,
            notes: Vec::new(),
            note_links: Vec::new(),
        }
    }

    pub fn spawn_prop(&mut self, pos: Vec3, shape: u8, half: f32, color: [f32; 3]) {
        let body = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(vector![pos.x, pos.y, pos.z])
                .angular_damping(0.4)
                .linear_damping(0.05)
                .build(),
        );
        let collider = if shape == 0 {
            ColliderBuilder::cuboid(half, half, half)
        } else {
            ColliderBuilder::ball(half)
        }
        .density(1.0)
        .friction(0.7)
        .restitution(0.35)
        .build();
        self.colliders
            .insert_with_parent(collider, body, &mut self.bodies);
        self.props.push(Prop {
            body,
            shape,
            half,
            color,
        });
    }

    /// Spawn one graph node: a gravity-free, heavily damped ball.
    pub fn spawn_note(&mut self, pos: Vec3, title: String, path: String) {
        let body = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(vector![pos.x, pos.y, pos.z])
                .gravity_scale(0.0)
                .linear_damping(2.5)
                .angular_damping(4.0)
                .build(),
        );
        let collider = ColliderBuilder::ball(0.26).density(0.4).build();
        self.colliders
            .insert_with_parent(collider, body, &mut self.bodies);
        self.notes.push(NoteNode { body, title, path });
    }

    pub fn clear_notes(&mut self) {
        for n in std::mem::take(&mut self.notes) {
            if self.grabbed.map(|(h, _)| h) == Some(n.body) {
                self.grabbed = None;
            }
            self.bodies.remove(
                n.body,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
        self.note_links.clear();
    }

    /// Force-directed layout (Notes System spec): wikilink springs pull,
    /// every pair repels, and a soft anchor holds the cloud over the stage.
    fn apply_graph_forces(&mut self) {
        if self.notes.is_empty() {
            return;
        }
        let positions: Vec<Vec3> = self
            .notes
            .iter()
            .filter_map(|n| {
                self.bodies
                    .get(n.body)
                    .map(|b| Vec3::new(b.translation().x, b.translation().y, b.translation().z))
            })
            .collect();
        if positions.len() != self.notes.len() {
            return;
        }
        let center = Vec3::new(0.0, 3.4, 0.0);
        let mut forces = vec![Vec3::ZERO; positions.len()];
        for i in 0..positions.len() {
            // Soft anchor to the cloud center.
            forces[i] += (center - positions[i]) * 0.6;
            // Pairwise repulsion (n ≤ ~100 notes: O(n²) is fine).
            for j in (i + 1)..positions.len() {
                let d = positions[i] - positions[j];
                let len2 = d.length_squared().max(0.05);
                let push = d / len2 * 1.4;
                forces[i] += push;
                forces[j] -= push;
            }
        }
        // Wikilink springs (rest length 1.6).
        for &(a, b) in &self.note_links {
            if a >= positions.len() || b >= positions.len() {
                continue;
            }
            let d = positions[b] - positions[a];
            let len = d.length().max(1e-3);
            let f = d / len * (len - 1.6) * 2.2;
            forces[a] += f;
            forces[b] -= f;
        }
        let grabbed = self.grabbed.map(|(h, _)| h);
        for (n, f) in self.notes.iter().zip(forces) {
            // The grabbed node belongs to the grab spring, not the layout.
            if grabbed == Some(n.body) {
                continue;
            }
            if let Some(body) = self.bodies.get_mut(n.body) {
                // Forces persist in rapier — reset, then apply this tick's.
                body.reset_forces(true);
                body.add_force(vector![f.x, f.y, f.z], true);
            }
        }
    }

    /// Advance simulation by real elapsed time (fixed-step accumulator).
    pub fn step(&mut self, elapsed: f32) {
        self.accumulator = (self.accumulator + elapsed).min(DT * MAX_STEPS_PER_FRAME as f32);
        while self.accumulator >= DT {
            self.accumulator -= DT;
            self.apply_graph_forces();
            self.pipeline.step(
                &vector![0.0, -9.81, 0.0],
                &self.integration,
                &mut self.islands,
                &mut self.broad_phase,
                &mut self.narrow_phase,
                &mut self.bodies,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.ccd,
                Some(&mut self.query),
                &(),
                &(),
            );
        }
    }

    /// Ray-pick a prop. Returns its handle and the hit distance.
    pub fn pick(&mut self, origin: Vec3, dir: Vec3) -> Option<(RigidBodyHandle, f32)> {
        self.query.update(&self.colliders);
        let ray = Ray::new(
            point![origin.x, origin.y, origin.z],
            vector![dir.x, dir.y, dir.z],
        );
        let (handle, toi) = self.query.cast_ray(
            &self.bodies,
            &self.colliders,
            &ray,
            100.0,
            true,
            QueryFilter::only_dynamic(),
        )?;
        let body = self.colliders.get(handle)?.parent()?;
        Some((body, toi))
    }

    pub fn grab(&mut self, body: RigidBodyHandle, depth: f32) {
        self.grabbed = Some((body, depth));
    }

    pub fn grab_depth(&self) -> Option<f32> {
        self.grabbed.map(|(_, d)| d)
    }

    pub fn grab_handle(&self) -> Option<(RigidBodyHandle, f32)> {
        self.grabbed
    }

    /// Pull the grabbed body toward `target` with a critically-damped spring
    /// (spec §3.4: kinematic spring attach, collisions still resolve).
    pub fn grab_move(&mut self, target: Vec3) {
        let Some((handle, _)) = self.grabbed else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            let pos = body.translation();
            let delta = Vec3::new(target.x - pos.x, target.y - pos.y, target.z - pos.z);
            let vel = body.linvel();
            let spring = delta * 45.0 - Vec3::new(vel.x, vel.y, vel.z) * 9.0;
            let mass = body.mass().max(0.05);
            body.reset_forces(true);
            body.add_force(vector![spring.x, spring.y, spring.z] * mass, true);
            body.wake_up(true);
        }
    }

    /// Release the grab; current velocity carries — that's the throw.
    /// Remove one spawned prop (stage syscalls, ABI 1.8).
    pub fn remove_prop(&mut self, index: usize) -> bool {
        if index >= self.props.len() {
            return false;
        }
        let prop = self.props.remove(index);
        if self.grabbed.map(|(h, _)| h) == Some(prop.body) {
            self.grabbed = None;
        }
        match self.focused {
            Some(f) if f == index => self.focused = None,
            Some(f) if f > index => self.focused = Some(f - 1),
            _ => {}
        }
        match self.hovered {
            Some(h) if h == index => self.hovered = None,
            Some(h) if h > index => self.hovered = Some(h - 1),
            _ => {}
        }
        self.bodies.remove(
            prop.body,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
        true
    }

    /// Resize a prop (non-dominant ✌ drag on the focused object): rebuilds
    /// its collider at the new half-extent.
    pub fn scale_prop(&mut self, index: usize, factor: f32) {
        let Some(prop) = self.props.get_mut(index) else {
            return;
        };
        let new_half = (prop.half * factor).clamp(0.15, 1.6);
        if (new_half - prop.half).abs() < 1e-4 {
            return;
        }
        prop.half = new_half;
        let shape = prop.shape;
        let body = prop.body;
        // Replace the collider: remove attached ones, insert fresh.
        let attached: Vec<_> = self
            .bodies
            .get(body)
            .map(|b| b.colliders().to_vec())
            .unwrap_or_default();
        for c in attached {
            self.colliders
                .remove(c, &mut self.islands, &mut self.bodies, false);
        }
        let collider = if shape == 0 {
            ColliderBuilder::cuboid(new_half, new_half, new_half)
        } else {
            ColliderBuilder::ball(new_half)
        }
        .restitution(0.4)
        .friction(0.8)
        .build();
        self.colliders
            .insert_with_parent(collider, body, &mut self.bodies);
        if let Some(b) = self.bodies.get_mut(body) {
            b.wake_up(true);
        }
    }

    pub fn clear_props(&mut self) {
        while !self.props.is_empty() {
            self.remove_prop(0);
        }
    }

    /// Push and/or spin a prop (stage syscalls) — physics does the animating.
    pub fn impulse_prop(&mut self, index: usize, imp: Vec3, torque: Vec3) -> bool {
        let Some(prop) = self.props.get(index) else {
            return false;
        };
        match self.bodies.get_mut(prop.body) {
            Some(body) => {
                body.apply_impulse(vector![imp.x, imp.y, imp.z], true);
                body.apply_torque_impulse(vector![torque.x, torque.y, torque.z], true);
                true
            }
            None => false,
        }
    }

    pub fn release(&mut self) {
        if let Some((handle, _)) = self.grabbed.take() {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.reset_forces(true);
            }
        }
    }

    /// Per-prop render data: position, rotation quaternion, shape, half, color.
    /// Stage props only (no graph nodes) — the ABI's StageList view.
    pub fn prop_instances(&self) -> Vec<([f32; 3], [f32; 4], u8, f32, [f32; 3])> {
        let mut all = self.instances();
        all.truncate(self.props.len());
        all
    }

    pub fn instances(&self) -> Vec<([f32; 3], [f32; 4], u8, f32, [f32; 3])> {
        self.props
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let body = self.bodies.get(p.body)?;
                let t = body.translation();
                let r = body.rotation();
                // First-class objects (UI spec §3.4): the one under the
                // pointer — or in the grip — glows brighter.
                let focused = self.focused == Some(i);
                let lit = self.hovered == Some(i)
                    || focused
                    || self.grabbed.map(|(h, _)| h) == Some(p.body);
                let boost = if focused { 1.9 } else { 1.5 };
                let color = if lit {
                    [
                        (p.color[0] * boost + 0.18).min(1.0),
                        (p.color[1] * boost + 0.18).min(1.0),
                        (p.color[2] * boost + 0.18).min(1.0),
                    ]
                } else {
                    p.color
                };
                Some((
                    [t.x, t.y, t.z],
                    [r.i, r.j, r.k, r.w],
                    p.shape,
                    p.half,
                    color,
                ))
            })
            .chain(self.notes.iter().filter_map(|n| {
                let body = self.bodies.get(n.body)?;
                let t = body.translation();
                let r = body.rotation();
                Some((
                    [t.x, t.y, t.z],
                    [r.i, r.j, r.k, r.w],
                    1u8, // sphere
                    0.26,
                    [0.55, 0.78, 1.0],
                ))
            }))
            .collect()
    }
}

impl Default for Physics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_props() -> Physics {
        let mut p = Physics::new();
        p.spawn_prop(Vec3::new(-2.0, 0.6, -1.0), 0, 0.5, [1.0, 0.5, 0.2]);
        p.spawn_prop(Vec3::new(2.2, 1.5, -0.8), 1, 0.5, [0.4, 0.9, 1.0]);
        p
    }

    #[test]
    fn props_fall_and_settle_on_the_floor() {
        let mut p = with_props();
        // Two simulated seconds.
        for _ in 0..120 {
            p.step(1.0 / 60.0);
        }
        for (pos, _, _, half, _) in p.instances() {
            assert!(
                pos[1] > half - 0.2 && pos[1] < 4.0,
                "prop should rest near the floor, got y = {}",
                pos[1]
            );
        }
    }

    #[test]
    fn pick_hits_a_prop_from_above() {
        let mut p = with_props();
        for _ in 0..240 {
            p.step(1.0 / 60.0);
        }
        // Ray straight down over the first prop.
        let (pos, ..) = p.instances()[0];
        let hit = p.pick(Vec3::new(pos[0], 10.0, pos[2]), Vec3::new(0.0, -1.0, 0.0));
        assert!(hit.is_some());
    }
}
