//! LLTinyGLTFHelper::initFetchedTextures: RGB technical maps and occlusion
//! packed into ORM red (Firestorm indra/newview, originally LGPL 2.1).

use crate::{AssetError, DecodedImage};

pub fn rgb(image: &DecodedImage) -> Result<DecodedImage, AssetError> {
    crate::import_texture::validate(image)?;
    if image.components == 3 {
        return Ok(image.clone());
    }
    let mut data = Vec::with_capacity(image.width as usize * image.height as usize * 3);
    for pixel in image.data.as_chunks::<4>().0 {
        data.extend_from_slice(&pixel[..3]);
    }
    Ok(DecodedImage {
        data,
        components: 3,
        width: image.width,
        height: image.height,
        discard: image.discard,
    })
}

/// A supplied occlusion map is resampled to ORM dimensions. A missing
/// occlusion input with `has_occlusion` retains an already packed red channel.
pub fn orm(mr: Option<&DecodedImage>, occlusion: Option<&DecodedImage>, has_occlusion: bool) -> Result<Option<DecodedImage>, AssetError> {
    let Some(source) = mr.or(occlusion) else { return Ok(None) };
    let mut result = if mr.is_some() {
        rgb(source)?
    } else {
        crate::import_texture::validate(source)?;
        DecodedImage {
            width: source.width,
            height: source.height,
            components: 3,
            data: vec![255; source.width as usize * source.height as usize * 3],
            discard: 0,
        }
    };
    if let Some(occlusion) = occlusion {
        let occlusion = rgb(occlusion)?;
        let data = if (occlusion.width, occlusion.height) == (result.width, result.height) {
            occlusion.data
        } else {
            let image = image::RgbImage::from_raw(occlusion.width, occlusion.height, occlusion.data)
                .ok_or_else(|| AssetError::invalid("occlusion image dimensions"))?;
            image::imageops::resize(&image, result.width, result.height, image::imageops::FilterType::Triangle).into_raw()
        };
        for (pixel, occlusion) in result.data.as_chunks_mut::<3>().0.iter_mut().zip(data.as_chunks::<3>().0) {
            pixel[0] = occlusion[0];
        }
    } else if !has_occlusion {
        for pixel in result.data.as_chunks_mut::<3>().0 {
            pixel[0] = 255;
        }
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixel(values: &[u8]) -> DecodedImage {
        DecodedImage {
            width: 1,
            height: 1,
            components: values.len() as u8,
            data: values.to_vec(),
            discard: 0,
        }
    }
    #[test]
    fn packs_occlusion_preserves_roughness_metal_and_removes_alpha() {
        let mr = pixel(&[8, 100, 220, 10]);
        let occlusion = pixel(&[64, 30, 20]);
        assert_eq!(
            orm(Some(&mr), Some(&occlusion), true).expect("pack").expect("map").data,
            [64, 100, 220]
        );
        assert_eq!(orm(Some(&mr), None, false).expect("pack").expect("map").data, [255, 100, 220]);
        assert_eq!(orm(Some(&mr), None, true).expect("pack").expect("map").data, [8, 100, 220]);
        assert_eq!(orm(None, Some(&occlusion), true).expect("pack").expect("map").data, [64, 255, 255]);
        assert_eq!(rgb(&mr).expect("RGB").components, 3);
    }
}
