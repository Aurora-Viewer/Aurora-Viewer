//! Prim data model: volume parameters, texture entries, extra parameters,
//! and procedural geometry generation (port of `llprimitive` and
//! `llmath/llvolume.cpp` from the Second Life viewer, originally LGPL 2.1).

pub mod extra;
mod pack;
pub mod params;
pub mod particles;
pub mod sculpt;
pub mod te;
pub mod volume;

pub use extra::{ExtraParams, LightParams, parse_extra_params};
pub use params::{PathParams, ProfileParams, RawShape, SculptParams, VolumeParams};
pub use sculpt::{generate_sculpt, sculpt_placeholder};
pub use te::{TextureEntry, TextureFace, parse_texture_entry};
pub use volume::{VolumeFace, VolumeMesh, generate_volume, lod_triangle_counts, num_faces};
