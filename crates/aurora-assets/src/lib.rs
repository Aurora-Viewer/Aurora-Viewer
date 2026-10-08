//! Asset decoders for the Aurora viewer.
//!
//! Ports of the Second Life / Firestorm viewer asset formats (originally LGPL 2.1):
//! - [`j2k`] - JPEG2000 textures (`llimage/llimagej2c.cpp`, `llimagej2coj/llimagej2coj.cpp`)
//! - [`mesh`] - mesh assets (`llmath/llvolume.cpp`, `llprimitive/llmodel.cpp`,
//!   `newview/llmeshrepository.cpp`)
//! - [`llm`] - avatar base meshes (`llappearance/llpolymesh.cpp`, `llappearance/llpolymorph.cpp`)
//! - [`skeleton`] - avatar skeleton `avatar_skeleton.xml` (`llappearance/llavatarappearance.cpp`, `llcharacter/lljoint.cpp`)
//! - [`material`] - GLTF material assets (`llprimitive/llgltfmaterial.cpp`, `newview/llgltfmateriallist.cpp`)
//!
//! Every decoder is pure (no global state), returns [`AssetError`] on malformed
//! input and never panics, so all of them can be called concurrently from rayon
//! worker threads.

pub mod anim;
pub mod j2k;
pub mod llm;
pub mod material;
pub mod mesh;
pub mod skeleton;

pub use anim::{Animation, JointMotion, parse_animation};
pub use j2k::{DecodedImage, J2kInfo, bytes_for_discard, decode_j2k, j2k_info};
pub use llm::{LlmMesh, LlmMorph, parse_llm};
pub use material::{AlphaMode, PbrMaterial, TextureTransform, parse_material_asset};
pub use mesh::{MeshFace, MeshHeader, SkinInfo, decode_mesh_lod, decode_skin, parse_mesh_header};
pub use skeleton::{Joint, Skeleton};

/// Error type shared by all asset decoders.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// The input ended before a required structure was complete.
    #[error("truncated asset data: {0}")]
    Truncated(&'static str),
    /// The input is structurally invalid.
    #[error("invalid asset data: {0}")]
    Invalid(String),
    /// The input is valid but uses a feature/version we do not handle.
    #[error("unsupported asset: {0}")]
    Unsupported(String),
    /// The input declares (or decompresses to) a size beyond our safety limits.
    #[error("asset exceeds size limit: {0}")]
    TooLarge(&'static str),
    /// zlib inflate failed.
    #[error("decompression failed: {0}")]
    Decompress(String),
    /// LLSD parse failure.
    #[error("llsd: {0}")]
    Llsd(#[from] aurora_llsd::LlsdError),
    /// XML parse failure.
    #[error("xml: {0}")]
    Xml(#[from] aurora_llsd::xml::XmlError),
    /// JSON parse failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// OpenJPEG failed to decode the codestream.
    #[error("jpeg2000 decode failed: {0}")]
    J2k(&'static str),
}

impl AssetError {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        AssetError::Invalid(msg.into())
    }
}

/// Little-endian bounds-checked reader shared by the binary decoders.
pub(crate) struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn seek(&mut self, pos: usize) -> Result<(), AssetError> {
        if pos > self.data.len() {
            return Err(AssetError::Truncated("seek past end"));
        }
        self.pos = pos;
        Ok(())
    }

    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], AssetError> {
        let end = self.pos.checked_add(n).ok_or(AssetError::Truncated(what))?;
        let s = self.data.get(self.pos..end).ok_or(AssetError::Truncated(what))?;
        self.pos = end;
        Ok(s)
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8, AssetError> {
        Ok(self.take(1, what)?[0])
    }

    pub(crate) fn u16(&mut self, what: &'static str) -> Result<u16, AssetError> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32, AssetError> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i32(&mut self, what: &'static str) -> Result<i32, AssetError> {
        Ok(self.u32(what)? as i32)
    }

    pub(crate) fn f32(&mut self, what: &'static str) -> Result<f32, AssetError> {
        Ok(f32::from_bits(self.u32(what)?))
    }

    pub(crate) fn vec2(&mut self, what: &'static str) -> Result<[f32; 2], AssetError> {
        Ok([self.f32(what)?, self.f32(what)?])
    }

    pub(crate) fn vec3(&mut self, what: &'static str) -> Result<[f32; 3], AssetError> {
        Ok([self.f32(what)?, self.f32(what)?, self.f32(what)?])
    }
}
