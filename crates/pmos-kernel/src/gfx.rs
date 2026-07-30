//! Graphics engine: wgpu device ownership and the per-frame render graph
//! (window textures → 3D stage → budgeted ray-trace compute → egui overlay).
//! Spec: docs/Architecture.md §4.1. Lands with milestone 2 (docs/Todo.md).
