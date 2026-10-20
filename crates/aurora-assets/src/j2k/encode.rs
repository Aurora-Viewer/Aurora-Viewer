//! Inventory thumbnails: lossless JPEG2000, as LLImageJ2COJ::encodeImpl.
//! OpenJPEG instances and output buffers belong to one encoding thread.

use super::*;
struct Writer {
    data: Vec<u8>,
    position: usize,
    maximum: usize,
}

unsafe extern "C" fn write(buffer: *mut c_void, size: usize, user: *mut c_void) -> usize {
    if user.is_null() || buffer.is_null() {
        return usize::MAX;
    }
    // SAFETY: OpenJPEG calls synchronously with our live Writer and readable buffer.
    let writer = unsafe { &mut *user.cast::<Writer>() };
    let Some(end) = writer.position.checked_add(size).filter(|end| *end <= writer.maximum) else {
        return usize::MAX;
    };
    if end > writer.data.len() {
        writer.data.resize(end, 0);
    }
    // SAFETY: OpenJPEG guarantees size readable bytes for this write callback.
    let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), size) };
    writer.data[writer.position..end].copy_from_slice(bytes);
    writer.position = end;
    size
}
unsafe extern "C" fn seek(offset: i64, user: *mut c_void) -> opj::OPJ_BOOL {
    if user.is_null() {
        return 0;
    }
    let Ok(position) = usize::try_from(offset) else {
        return 0;
    };
    // SAFETY: the stream holds the Writer alive until compression ends.
    let writer = unsafe { &mut *user.cast::<Writer>() };
    if position > writer.maximum {
        return 0;
    }
    writer.position = position;
    if position > writer.data.len() {
        writer.data.resize(position, 0);
    }
    1
}
unsafe extern "C" fn skip(offset: i64, user: *mut c_void) -> i64 {
    if user.is_null() {
        return -1;
    }
    // SAFETY: the stream holds the Writer alive until compression ends.
    let position = unsafe { (*user.cast::<Writer>()).position };
    let Some(next) = (position as i64).checked_add(offset) else {
        return -1;
    };
    // SAFETY: seek receives the same live Writer registered with the stream.
    if unsafe { seek(next, user) } == 0 { -1 } else { offset }
}

/// Square, power-of-two RGB/RGBA thumbnails, 64–256 px (LLFloaterSimpleSnapshot).
pub fn encode_thumbnail_j2k(width: u32, height: u32, components: u8, data: &[u8]) -> Result<Vec<u8>, AssetError> {
    if width != height
        || !(64..=256).contains(&width)
        || !width.is_power_of_two()
        || !matches!(components, 3 | 4)
        || data.len() != width as usize * height as usize * components as usize
    {
        return Err(AssetError::invalid("invalid thumbnail dimensions or pixels"));
    }
    encode(width, height, components, data, true, 2 * 1024 * 1024)
}

/// LLImageJ2COJ::encodeImpl: reversible for small uploads, otherwise
/// progressive quality layers with an 8:1 final compression ratio.
/// Pixels and codestream rows are top-to-bottom, like the decoder. Callers
/// using DecodedImage must keep its row order when preparing an upload.
pub fn encode_texture_j2k(width: u32, height: u32, components: u8, data: &[u8], lossless: bool) -> Result<Vec<u8>, AssetError> {
    encode(width, height, components, data, lossless, 32 * 1024 * 1024)
}
fn encode(width: u32, height: u32, components: u8, data: &[u8], lossless: bool, maximum: usize) -> Result<Vec<u8>, AssetError> {
    if !(4..=2048).contains(&width)
        || !(4..=2048).contains(&height)
        || !width.is_power_of_two()
        || !height.is_power_of_two()
        || !matches!(components, 3 | 4)
        || data.len() != width as usize * height as usize * components as usize
    {
        return Err(AssetError::invalid("invalid texture dimensions or pixels"));
    }
    // SAFETY: sizes are bounded above, every allocation is checked, guards
    // release each native object once, callbacks borrow the live local Writer.
    unsafe {
        let mut params: opj::opj_cparameters_t = std::mem::zeroed();
        opj::opj_set_default_encoder_parameters(&mut params);
        let layers = if lossless {
            1
        } else {
            match width * height {
                0..=1024 => 2,
                1025..=16384 => 3,
                16385..=262144 => 4,
                262145..=1048576 => 5,
                _ => 6,
            }
        };
        params.tcp_numlayers = layers;
        for i in 0..layers {
            params.tcp_rates[i as usize] = if lossless {
                0.0
            } else {
                8.0 * (1u32 << ((layers - i - 1) * 2)) as f32
            };
        }
        params.cp_disto_alloc = 1;
        params.irreversible = i32::from(!lossless);
        params.tcp_mct = 1;
        params.numresolution = params.numresolution.min(1 + width.min(height).ilog2() as i32);
        let mut planes: Vec<opj::opj_image_cmptparm_t> = (0..components)
            .map(|_| {
                let mut p: opj::opj_image_cmptparm_t = std::mem::zeroed();
                p.dx = 1;
                p.dy = 1;
                p.w = width;
                p.h = height;
                p.prec = 8;
                p
            })
            .collect();
        let image = ImageGuard(opj::opj_image_create(
            components.into(),
            planes.as_mut_ptr(),
            opj::COLOR_SPACE::OPJ_CLRSPC_SRGB,
        ));
        if image.0.is_null() {
            return Err(AssetError::J2k("image allocation failed"));
        }
        (*image.0).x1 = width;
        (*image.0).y1 = height;
        for c in 0..usize::from(components) {
            let plane = &*(*image.0).comps.add(c);
            if plane.data.is_null() {
                return Err(AssetError::J2k("component allocation failed"));
            }
            let output = std::slice::from_raw_parts_mut(plane.data, width as usize * height as usize);
            for (n, pixel) in output.iter_mut().enumerate() {
                *pixel = data[n * usize::from(components) + c].into();
            }
        }
        let codec = CodecGuard(opj::opj_create_compress(opj::CODEC_FORMAT::OPJ_CODEC_J2K));
        if codec.0.is_null() {
            return Err(AssetError::J2k("encoder allocation failed"));
        }
        if opj::opj_setup_encoder(codec.0, &mut params, image.0) == 0 {
            return Err(AssetError::J2k("encoder setup failed"));
        }
        let mut writer = Writer {
            data: Vec::new(),
            position: 0,
            maximum,
        };
        // Destroy the stream before its borrowed writer goes out of scope.
        let stream = StreamGuard(opj::opj_stream_create(1 << 16, 0));
        if stream.0.is_null() {
            return Err(AssetError::J2k("stream allocation failed"));
        }
        opj::opj_stream_set_user_data(stream.0, (&mut writer as *mut Writer).cast(), None);
        opj::opj_stream_set_write_function(stream.0, Some(write));
        opj::opj_stream_set_skip_function(stream.0, Some(skip));
        opj::opj_stream_set_seek_function(stream.0, Some(seek));
        if opj::opj_start_compress(codec.0, image.0, stream.0) == 0
            || opj::opj_encode(codec.0, stream.0) == 0
            || opj::opj_end_compress(codec.0, stream.0) == 0
        {
            return Err(AssetError::J2k("image encoding failed"));
        }
        drop(stream);
        Ok(writer.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnails_round_trip_without_losing_color_or_alpha() {
        for components in [3, 4] {
            let pixels: Vec<_> = (0..64 * 64 * usize::from(components)).map(|i| (i % 251) as u8).collect();
            let encoded = encode_thumbnail_j2k(64, 64, components, &pixels).expect("encode");
            let decoded = decode_j2k(&encoded, 0).expect("decode");
            assert_eq!(decoded.components, components);
            assert_eq!(decoded.data, pixels);
        }
        assert!(encode_thumbnail_j2k(64, 64, 4, &[0; 8]).is_err());
        assert!(encode_thumbnail_j2k(1024, 1024, 4, &[]).is_err());
    }
}
