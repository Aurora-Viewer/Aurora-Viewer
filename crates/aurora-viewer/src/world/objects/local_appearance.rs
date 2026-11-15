//! Client-only bitmap channels (LLLocalBitmap::updateUserPrims,
//! updateUserVolumes and updateGLTFMaterials, indra/newview/lllocalbitmaps.cpp).
//! Keep the simulator's texture entries and material overrides untouched.

use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub enum LocalTextureSlot {
    Diffuse,
    Material,
    Normal,
    Specular,
    Pbr(usize),
    Light,
}

impl LocalTextureSlot {
    fn index(self) -> usize {
        match self {
            Self::Diffuse => 0,
            Self::Material => 1,
            Self::Normal => 2,
            Self::Specular => 3,
            Self::Pbr(channel) => 4 + channel.min(3),
            Self::Light => 8,
        }
    }
}

/// Preserve the existing registry representation for single surface bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LocalAppearance {
    Texture(Uuid),
    Material(Uuid),
    Channels([Uuid; 9]),
}

impl LocalAppearance {
    fn channels(self) -> [Uuid; 9] {
        let mut channels = [Uuid::nil(); 9];
        match self {
            Self::Texture(id) => channels[0] = id,
            Self::Material(id) => channels[1] = id,
            Self::Channels(values) => channels = values,
        }
        channels
    }

    fn from_channels(channels: [Uuid; 9]) -> Option<Self> {
        if channels[2..].iter().any(|id| !id.is_nil()) {
            Some(Self::Channels(channels))
        } else if !channels[1].is_nil() {
            Some(Self::Material(channels[1]))
        } else if !channels[0].is_nil() {
            Some(Self::Texture(channels[0]))
        } else {
            None
        }
    }

    pub fn texture(self, slot: LocalTextureSlot) -> Option<Uuid> {
        let id = self.channels()[slot.index()];
        (!id.is_nil()).then_some(id)
    }

    pub fn surface(self) -> Option<Self> {
        self.texture(LocalTextureSlot::Material)
            .map(Self::Material)
            .or_else(|| self.texture(LocalTextureSlot::Diffuse).map(Self::Texture))
    }

    pub fn with_texture(current: Option<Self>, slot: LocalTextureSlot, texture: Option<Uuid>) -> Option<Self> {
        let mut channels = current.map_or([Uuid::nil(); 9], Self::channels);
        let index = slot.index();
        if index < 2 {
            // Diffuse and the full material replace one another, as before.
            channels[0] = Uuid::nil();
            channels[1] = Uuid::nil();
        }
        channels[index] = texture.unwrap_or_default();
        Self::from_channels(channels)
    }

    pub fn entries(self) -> impl Iterator<Item = (Uuid, bool)> {
        self.channels()
            .into_iter()
            .enumerate()
            .filter_map(|(slot, id)| (!id.is_nil()).then_some((id, slot == 1)))
    }

    pub fn restore(current: Option<Self>, slot: LocalTextureSlot, original: Option<Self>) -> Option<Self> {
        let mut channels = current.map_or([Uuid::nil(); 9], Self::channels);
        let old = original.map_or([Uuid::nil(); 9], Self::channels);
        let index = slot.index();
        if index < 2 {
            channels[..2].copy_from_slice(&old[..2]);
        } else {
            channels[index] = old[index];
        }
        Self::from_channels(channels)
    }

    pub fn contains(self, entry: Uuid) -> bool {
        self.entries().any(|(id, _)| id == entry)
    }

    pub fn retain(self, mut keep: impl FnMut(Uuid) -> bool) -> Option<Self> {
        Self::from_channels(self.channels().map(|id| if id.is_nil() || keep(id) { id } else { Uuid::nil() }))
    }

    #[cfg(test)]
    pub fn entry(self) -> Uuid {
        self.entries().next().map_or(Uuid::nil(), |(id, _)| id)
    }

    pub fn apply_pbr(self, material: &mut aurora_assets::PbrMaterial) {
        for (channel, texture) in [
            &mut material.base_color_texture,
            &mut material.normal_texture,
            &mut material.metallic_roughness_texture,
            &mut material.emissive_texture,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(id) = self.texture(LocalTextureSlot::Pbr(channel)) {
                *texture = Some(id);
            }
        }
    }

    pub fn apply_legacy(self, material: &mut crate::scene::legacy_mat::LegacyMaterial) {
        if let Some(id) = self.texture(LocalTextureSlot::Normal) {
            material.normal_map = id;
        }
        if let Some(id) = self.texture(LocalTextureSlot::Specular) {
            material.specular_map = id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_map_rendering_preserves_material_properties_and_other_channels() {
        let normal = Uuid::from_u128(20);
        let specular = Uuid::from_u128(21);
        let base = Uuid::from_u128(22);
        let value = LocalAppearance::with_texture(None, LocalTextureSlot::Normal, Some(normal));
        let value = LocalAppearance::with_texture(value, LocalTextureSlot::Specular, Some(specular));
        let value = LocalAppearance::with_texture(value, LocalTextureSlot::Pbr(0), Some(base)).unwrap();
        let mut legacy = crate::scene::legacy_mat::LegacyMaterial {
            normal_st: [2.0, 3.0, 0.2, 0.3],
            specular_exp: 100,
            ..Default::default()
        };
        value.apply_legacy(&mut legacy);
        assert_eq!((legacy.normal_map, legacy.specular_map), (normal, specular));
        assert_eq!(legacy.normal_st, [2.0, 3.0, 0.2, 0.3]);
        assert_eq!(legacy.specular_exp, 100);
        let mut pbr = aurora_assets::PbrMaterial {
            roughness_factor: 0.35,
            ..Default::default()
        };
        value.apply_pbr(&mut pbr);
        assert_eq!(pbr.base_color_texture, Some(base));
        assert_eq!(pbr.normal_texture, None);
        assert_eq!(pbr.roughness_factor, 0.35);
    }

    #[test]
    fn local_channels_coexist_and_removing_one_file_preserves_the_others() {
        let diffuse = Uuid::from_u128(1);
        let normal = Uuid::from_u128(2);
        let specular = Uuid::from_u128(3);
        let initial = Some(LocalAppearance::Texture(diffuse));
        let value = LocalAppearance::with_texture(initial, LocalTextureSlot::Normal, Some(normal));
        let value = LocalAppearance::with_texture(value, LocalTextureSlot::Specular, Some(specular)).unwrap();
        assert_eq!(value.surface(), initial);
        let value = value.retain(|id| id != normal).unwrap();
        assert_eq!(value.texture(LocalTextureSlot::Normal), None);
        assert_eq!(value.texture(LocalTextureSlot::Specular), Some(specular));
        assert_eq!(value.retain(|id| id != specular), initial);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<LocalAppearance>(&json).unwrap(), value);
    }
}
