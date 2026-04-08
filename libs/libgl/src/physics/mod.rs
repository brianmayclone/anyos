// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Rigid-body physics engine for libgl.
//!
//! The implementation is intentionally compact, but split into focused
//! modules so world stepping, contact generation, rigid-body definitions and
//! math helpers can evolve independently.

mod body;
mod contact;
mod math;
mod narrow;
mod world;

pub use body::{Collider, RigidBody};
pub use math::{Quat, Vec3};
pub use world::PhysicsWorld;
