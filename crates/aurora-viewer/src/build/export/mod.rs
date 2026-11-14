//! Object › Save as › Collada / OBJ: the selected objects written to a 3D
//! file for an outside tool, with their textures beside it. Port of the
//! Legacy Viewer's Collada export (ColladaExportFloater and DAESaver,
//! indra/newview/daeexport.cpp, Latif Khalifa and Cinder Biscuits; its
//! rights, FSExportPermsCheck, indra/newview/fsexportperms.cpp, Cinder
//! Biscuits; LGPL 2.1). OBJ is Aurora's own, from the same model.
//!
//! - `rights.rs`: which prims and textures may be written.
//! - `gather.rs`: the selection's prims, their facts and geometry, read from
//!   the world and the scene.
//! - `model.rs`: the faces gathered per prim, with materials and options.
//! - `collada.rs`, `obj.rs`: the file formats.
//! - `textures.rs`: the texture files beside the model.
//!
//! The window is `build/ui/export.rs`, the steps on the app's side
//! `app/object_export.rs`.

pub mod collada;
pub mod gather;
pub mod model;
pub mod obj;
pub mod rights;
pub mod textures;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// The file written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Format {
    #[default]
    Collada,
    Obj,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Collada => "dae",
            Format::Obj => "obj",
        }
    }
}

/// DAEExportTexturesFormat: the texture files' format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageFormat {
    /// TARGA (0).
    Tga,
    /// PNG (1), the default.
    #[default]
    Png,
    /// JPEG2000 (2): the texture as the grid sends it, not decoded.
    J2c,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 3] = [ImageFormat::Tga, ImageFormat::Png, ImageFormat::J2c];

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Tga => "tga",
            ImageFormat::Png => "png",
            ImageFormat::J2c => "j2c",
        }
    }

    /// The combo's label (floater_export_collada.xml).
    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Tga => "TARGA",
            ImageFormat::Png => "PNG",
            ImageFormat::J2c => "JPEG2000",
        }
    }
}

/// The export options, kept between sessions (the DAEExport* settings,
/// all on by default as in the Legacy Viewer's settings.xml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The last format chosen.
    pub format: Format,
    /// DAEExportTextures: write the textures we may beside the file.
    pub textures: bool,
    /// DAEExportTexturesFormat.
    pub texture_format: ImageFormat,
    /// DAEExportConsolidateMaterials: one material per texture and colour.
    pub consolidate: bool,
    /// DAEExportSkipTransparent: leave out invisible faces.
    pub skip_transparent: bool,
    /// DAEExportTextureParams: repeats, offset and rotation applied to the
    /// texture coordinates.
    pub texture_params: bool,
    /// DAEExportSingleUVMap: every prim's texture coordinates under one
    /// name (Collada only).
    pub single_uv_map: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Collada,
            textures: true,
            texture_format: ImageFormat::Png,
            consolidate: true,
            skip_transparent: true,
            texture_params: true,
            single_uv_map: true,
        }
    }
}

/// The textures written beside the model: their file names (without the
/// extension), each one unique, and the extension of their format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextureFiles {
    pub names: HashMap<Uuid, String>,
    pub extension: String,
}

impl TextureFiles {
    /// Name each texture that may be written after its item
    /// (`rights::texture`); two textures of the same name get "_2", "_3"…
    /// (the Legacy Viewer wrote one over the other).
    pub fn new(named: impl IntoIterator<Item = (Uuid, String)>, format: ImageFormat) -> Self {
        let mut named: Vec<(Uuid, String)> = named.into_iter().collect();
        named.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        let mut names = HashMap::new();
        let mut taken = std::collections::HashSet::new();
        for (id, name) in named {
            let mut unique = name.clone();
            let mut n = 2;
            while !taken.insert(unique.to_lowercase()) {
                unique = format!("{name}_{n}");
                n += 1;
            }
            names.insert(id, unique);
        }
        Self {
            names,
            extension: format.extension().into(),
        }
    }

    /// The names in a fixed order, for the file's list of images.
    pub fn sorted_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.names.values().map(String::as_str).collect();
        names.sort_unstable();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_names_are_unique_whatever_their_case() {
        let files = TextureFiles::new(
            [
                (Uuid::from_u128(1), "Wood".to_owned()),
                (Uuid::from_u128(2), "wood".to_owned()),
                (Uuid::from_u128(3), "Wood".to_owned()),
                (Uuid::from_u128(4), "Stone".to_owned()),
            ],
            ImageFormat::Tga,
        );
        assert_eq!(files.extension, "tga");
        assert_eq!(files.names[&Uuid::from_u128(1)], "Wood");
        assert_eq!(files.names[&Uuid::from_u128(3)], "Wood_2");
        assert_eq!(files.names[&Uuid::from_u128(2)], "wood_3");
        assert_eq!(files.sorted_names(), ["Stone", "Wood", "Wood_2", "wood_3"]);
    }

    #[test]
    fn the_legacy_defaults() {
        let s: Settings = serde_json::from_str("{}").expect("defaults");
        assert_eq!(s, Settings::default());
        assert!(s.textures && s.consolidate && s.skip_transparent && s.texture_params && s.single_uv_map);
        assert_eq!(s.texture_format, ImageFormat::Png);
    }
}
