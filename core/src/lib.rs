//! squaremap for Horizon: the parts that need no game types. The game glue (hooks and
//! chunk snapshots) lives in the bundle's helpers and support module.
//!
//! Ported from squaremap (https://github.com/jpenilla/squaremap), MIT,
//! Copyright (c) 2021-2024 Jason Penilla, Copyright (c) 2020-2021 William Blake Galbreath.
pub mod api;
pub mod assets;
pub mod color;
pub mod commands;
pub mod config;
pub mod image;
pub mod json;
pub mod nbt;
pub mod region;
pub mod render;
pub mod runtime;
pub mod scan;
pub mod tables;
pub mod vanilla;
pub mod visibility;
pub mod web;

pub use json::Player;
pub use runtime::*;
pub use region::WorldEnv;
pub use scan::{ChunkView, SwampGrid};
pub use tables::{grass_modifier_after_tint, Fluid, GrassModifier, RawBiome, RawState};
