//! Aurora Weather: the detail of the snow lying on the ground ("Snow 004",
//! ambientCG, CC0; assets/snow/README.md): a normal and height map, and a
//! colour and roughness map, uploaded once as bindless textures with their
//! mips, their slots given to the wet ground pass (`RainLook::snow_detail`:
//! the first, plus the second times 4096).

use aurora_render::{MipLevel, Renderer};

const PNG: &[u8] = include_bytes!("../../../../assets/snow/snow-normal-height.png");
const COLOR_PNG: &[u8] = include_bytes!("../../../../assets/snow/snow-color-roughness.png");
/// The slots' packing in `RainLook::snow_detail` (env.wgsl `snow_detail_slot`).
const PACK: u32 = 4096;

/// The textures' slots, made on first use (None if they could not be).
#[derive(Default)]
pub struct SnowDetail {
    slot: Option<u32>,
    tried: bool,
}

impl SnowDetail {
    /// The normal and height map's slot, plus the colour and roughness
    /// map's times 4096 (none of the second when it could not be made, or
    /// a slot is past the packing).
    pub fn slot(&mut self, renderer: &mut Renderer) -> Option<u32> {
        if !self.tried {
            self.tried = true;
            let detail = upload(renderer, PNG, "snow detail texture");
            let color = upload(renderer, COLOR_PNG, "snow colour texture");
            self.slot = detail.map(|d| match color {
                Some(c) if d < PACK && c < PACK => d + c * PACK,
                _ => d,
            });
        }
        self.slot
    }
}

fn upload(renderer: &mut Renderer, png: &[u8], what: &str) -> Option<u32> {
    let image = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            log::warn!("{what}: {e}");
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
        log::warn!("{what}: no slot");
    }
    slot
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_snow_detail_is_a_square_rgba_png() {
        for png in [super::PNG, super::COLOR_PNG] {
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("decodes");
            assert_eq!((image.width(), image.height()), (512, 512));
            assert_eq!(image.color(), image::ColorType::Rgba8);
        }
    }
}
