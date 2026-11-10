//! The face-select pattern of the build tool's Select face mode: which faces
//! show it, and its texture, the grid asset IMG_FACE_SELECT.
//!
//! Port of LLPipeline::renderSelectedFaces (indra/newview/pipeline.cpp,
//! originally LGPL 2.1): in TE mode (LLSelectMgr::getTEMode) every selected
//! face is drawn again with that texture, fetched from the grid and kept at
//! full resolution (`addTextureStats(MAX_IMAGE_AREA)`). The faces picked
//! otherwise (a local appearance shown, a texture dragged over a face, the
//! pipette, photo or give marks) keep the coverage halo alone: they are not
//! TE mode. The renderer draws the pattern (aurora-render selection.rs);
//! while the texture loads, and offline, its shader draws a stand-in.

use super::Scene;
use crate::scene::textures::{TexSource, TextureStreamer};
use aurora_render::Renderer;
use std::collections::HashMap;
use uuid::Uuid;

/// IMG_FACE_SELECT (indra/llcommon/indra_constants.cpp): "face selector".
pub const IMG_FACE_SELECT: Uuid = Uuid::from_u128(0xa85ac674_cb75_4af6_9499_df7c5aaf7a28);

/// Whether face `te` of object `idx` is in `faces` (one bit per TE).
#[inline]
pub fn has_face(faces: &HashMap<usize, u64>, idx: usize, te: u8) -> bool {
    faces
        .get(&idx)
        .is_some_and(|mask| mask & 1u64.checked_shl(te.into()).unwrap_or(0) != 0)
}

/// The pattern's texture: acquired the first time Select face mode is used,
/// then held, like the Legacy Viewer's `mFaceSelectImagep`, so that it is
/// ready at the next face picked.
#[derive(Default)]
pub struct Texture {
    held: bool,
}

impl Texture {
    /// The slot to draw the pattern with: the texture's once loaded, 0 (the
    /// shader's stand-in) until then. `wanted`: Select face mode is on.
    pub fn slot(&mut self, textures: &mut TextureStreamer, renderer: &mut Renderer, wanted: bool) -> u32 {
        if !wanted {
            return 0;
        }
        if !self.held {
            // one reference, kept; its slot is read afresh each frame (a
            // texture met with the table full gets its slot later)
            textures.acquire(renderer, IMG_FACE_SELECT, TexSource::Asset);
            self.held = true;
        }
        // never lowered by memory pressure, at full resolution
        textures.note_usage_fixed(&IMG_FACE_SELECT, f32::MAX);
        if textures.is_loaded(&IMG_FACE_SELECT) || textures.is_local(&IMG_FACE_SELECT) {
            textures.slot_of(&IMG_FACE_SELECT).unwrap_or(0)
        } else {
            0
        }
    }

    /// The GPU is lost: the texture is released with the rest, and acquired
    /// again on the new renderer when the mode is next used.
    pub fn forget(&mut self, textures: &mut TextureStreamer) {
        if std::mem::take(&mut self.held) {
            textures.release(&IMG_FACE_SELECT);
        }
    }
}

impl Scene {
    /// The faces showing the face-select pattern: the faces picked in
    /// Select face mode, whether or not "Show highlight" draws the halo
    /// (LLPipeline::renderHighlights, TE mode only).
    pub fn set_face_pattern(&mut self, faces: &HashMap<usize, u64>) {
        if self.pattern_faces != *faces {
            self.pattern_faces.clone_from(faces);
        }
    }

    /// The pattern texture's slot for this frame (`Texture::slot`).
    pub fn face_select_texture(&mut self, renderer: &mut Renderer, wanted: bool) -> u32 {
        self.face_pattern.slot(&mut self.textures, renderer, wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_face_reads_one_bit_per_texture_entry() {
        let faces = HashMap::from([(3usize, 0b1010u64), (7, 1u64 << 44)]);
        assert!(has_face(&faces, 3, 1));
        assert!(has_face(&faces, 3, 3));
        assert!(!has_face(&faces, 3, 0));
        assert!(!has_face(&faces, 3, 2));
        assert!(has_face(&faces, 7, 44));
        assert!(!has_face(&faces, 4, 1), "another object");
        // past the mask: never a face, never a panic
        assert!(!has_face(&faces, 3, 64));
        assert!(!has_face(&faces, 3, 255));
    }

    #[test]
    fn face_select_asset_is_the_legacy_constant() {
        assert_eq!(IMG_FACE_SELECT.to_string(), "a85ac674-cb75-4af6-9499-df7c5aaf7a28");
    }
}
