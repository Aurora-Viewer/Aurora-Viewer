//! Enhanced water's foam texture: "Foam 003" from ambientCG (CC0;
//! assets/water/README.md), a lace of sea foam and its thickness, uploaded
//! once as a bindless texture with its mips; its slot goes to the water's
//! shaders (`RainLook::water_foam`, common.wgsl `water_foam_lace`), which
//! show as much of it as there is foam: thick foam fills its cells, thin
//! foam leaves its strands.

use aurora_render::{MipLevel, Renderer};

const PNG: &[u8] = include_bytes!("../../../../assets/water/foam-lace-height.png");

/// The texture's slot, made on first use (None if it could not be).
#[derive(Default)]
pub struct WaterFoam {
    slot: Option<u32>,
    tried: bool,
}

impl WaterFoam {
    pub fn slot(&mut self, renderer: &mut Renderer) -> Option<u32> {
        if !self.tried {
            self.tried = true;
            self.slot = upload(renderer);
        }
        self.slot
    }
}

fn upload(renderer: &mut Renderer) -> Option<u32> {
    // (grey and alpha: the lace in r, g and b, its thickness in a)
    let image = match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            log::warn!("water foam texture: {e}");
            return None;
        }
    };
    let mut levels = vec![image];
    while levels.last().is_some_and(|l| l.width() > 1) {
        let last = levels.last().expect("a level");
        let next = image::imageops::resize(
            last,
            (last.width() / 2).max(1),
            (last.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
        levels.push(next);
    }
    let mips: Vec<MipLevel> = levels
        .iter()
        .map(|l| MipLevel {
            width: l.width(),
            height: l.height(),
            data: l.as_raw(),
        })
        .collect();
    let slot = renderer.create_texture(&mips);
    if slot.is_none() {
        log::warn!("water foam texture: no slot");
    }
    slot
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_foam_is_a_square_grey_and_alpha_png() {
        let image = image::load_from_memory_with_format(super::PNG, image::ImageFormat::Png).expect("decodes");
        assert_eq!((image.width(), image.height()), (1024, 1024));
        assert_eq!(image.color(), image::ColorType::La8);
    }
}
