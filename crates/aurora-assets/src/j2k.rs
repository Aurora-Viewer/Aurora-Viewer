//! JPEG2000 texture decoding.
//!
//! Ported from the Second Life / Firestorm viewer sources (originally LGPL 2.1):
//! - `indra/llimage/llimagej2c.cpp` (`LLImageJ2C::calcDataSizeJ2C`, `calcHeaderSizeJ2C`)
//! - `indra/llimagej2coj/llimagej2coj.cpp` (`JPEG2KDecode`, memory stream callbacks,
//!   `LLImageJ2COJ::decodeImpl`)
//!
//! Copyright (C) 2006-2024, Linden Research, Inc. and the Firestorm project.
//!
//! Differences from the C++ code:
//! - Output rows are stored **top-to-bottom** in codestream order. `LLImageRaw`
//!   stores rows bottom-up (GL convention); a renderer with a top-left texture
//!   origin (wgpu/D3D/Metal) can upload [`DecodedImage::data`] as-is and sample
//!   SL texture coordinates with `v' = 1 - v`.
//! - Components with precision != 8, signed samples and subsampled components
//!   are converted to 8-bit full-resolution samples (LL assumes 8-bit unsigned).
//! - At most 4 components are returned (LL's `max_channel_count`).

use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use openjpeg_sys as opj;

use crate::AssetError;
mod encode;
pub use encode::encode_thumbnail_j2k;

/// `MAX_DISCARD_LEVEL` from `llimage.h`.
pub const MAX_DISCARD_LEVEL: u8 = 5;
/// `FIRST_PACKET_SIZE` from `llimage.h`, returned by `LLImageJ2C::calcHeaderSizeJ2C`.
pub const HEADER_SIZE_ESTIMATE: usize = 600;
/// `MAX_BLOCK_SIZE` from `llimage.h`.
const MAX_BLOCK_SIZE: i64 = 64;
/// `DEFAULT_COMPRESSION_RATE` from `llimagej2c.h`.
const DEFAULT_COMPRESSION_RATE: f32 = 1.0 / 8.0;

/// Sanity limits applied before handing a codestream to OpenJPEG, so that a
/// malicious header cannot request gigantic allocations.
const MAX_DIMENSION: u32 = 16384;
const MAX_PIXELS: u64 = 8192 * 8192;
const MAX_HEADER_COMPONENTS: u16 = 16;

const JP2_SIGNATURE: [u8; 12] = [0, 0, 0, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// Basic codestream parameters read from the SIZ/COD marker segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct J2kInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    /// Number of wavelet decomposition levels (COD `SPcod` NL); the maximum
    /// usable discard level (`cp_reduce`).
    pub levels: u8,
}

/// A decoded image: tightly packed 8-bit samples, `components` bytes per pixel,
/// rows top-to-bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub data: Vec<u8>,
    /// Resolution reduction actually applied (each level halves both dimensions).
    pub discard: u8,
}

/// Locate the raw J2K codestream: either the data itself (SL textures) or the
/// contents of the `jp2c` box of a JP2 file.
fn locate_codestream(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(&[0xFF, 0x4F]) {
        return Some(data);
    }
    if !data.starts_with(&JP2_SIGNATURE) {
        return None;
    }
    let mut pos = 0usize;
    loop {
        let hdr = data.get(pos..pos.checked_add(8)?)?;
        let lbox = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let tbox = &hdr[4..8];
        let (header_len, box_len) = match lbox {
            0 => (8u64, (data.len() - pos) as u64),
            1 => {
                let x = data.get(pos + 8..pos.checked_add(16)?)?;
                let mut b = [0u8; 8];
                b.copy_from_slice(x);
                (16u64, u64::from_be_bytes(b))
            }
            n => (8u64, n),
        };
        if box_len < header_len {
            return None;
        }
        let content_start = pos.checked_add(header_len as usize)?;
        if tbox == b"jp2c" {
            let end = pos.saturating_add(usize::try_from(box_len).unwrap_or(usize::MAX)).min(data.len());
            return data.get(content_start..end);
        }
        pos = pos.checked_add(usize::try_from(box_len).ok()?)?;
        if pos >= data.len() {
            return None;
        }
    }
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parse SIZ and COD from a raw codestream main header.
fn parse_main_header(cs: &[u8]) -> Option<J2kInfo> {
    if be16(cs, 0)? != 0xFF4F {
        return None;
    }
    let mut pos = 2usize;
    let mut size: Option<(u32, u32, u16)> = None;
    let mut levels: Option<u8> = None;
    while size.is_none() || levels.is_none() {
        let marker = be16(cs, pos)?;
        if marker >> 8 != 0xFF {
            return None;
        }
        // SOT / SOD / EOC: end of the main header.
        if matches!(marker, 0xFF90 | 0xFF93 | 0xFFD9) {
            break;
        }
        let seg_len = be16(cs, pos + 2)? as usize;
        if seg_len < 2 {
            return None;
        }
        let seg = cs.get(pos + 4..pos + 2 + seg_len)?;
        match marker {
            0xFF51 => {
                // SIZ: Rsiz(2) Xsiz Ysiz XOsiz YOsiz XTsiz YTsiz XTOsiz YTOsiz (4 each) Csiz(2)
                let xsiz = be32(seg, 2)?;
                let ysiz = be32(seg, 6)?;
                let xosiz = be32(seg, 10)?;
                let yosiz = be32(seg, 14)?;
                let csiz = be16(seg, 34)?;
                let w = xsiz.checked_sub(xosiz)?;
                let h = ysiz.checked_sub(yosiz)?;
                if w == 0 || h == 0 || csiz == 0 {
                    return None;
                }
                size = Some((w, h, csiz));
            }
            0xFF52 => {
                // COD: Scod(1) prog(1) layers(2) mct(1) NL(1) ...
                levels = Some(*seg.get(5)?);
            }
            _ => {}
        }
        pos = pos.checked_add(2 + seg_len)?;
    }
    let (width, height, comps) = size?;
    Some(J2kInfo {
        width,
        height,
        components: comps.min(255) as u8,
        levels: levels?,
    })
}

/// Read image dimensions, component count and decomposition levels from the
/// codestream main header without decoding. Returns `None` if the header is
/// missing, incomplete (SIZ and COD must both be present) or malformed.
pub fn j2k_info(data: &[u8]) -> Option<J2kInfo> {
    parse_main_header(locate_codestream(data)?)
}

/// Port of `LLImageJ2C::calcDataSizeJ2C` (Firestorm FIRE-35987 variant, rate
/// `DEFAULT_COMPRESSION_RATE`): estimated number of bytes, including the
/// `FIRST_PACKET_SIZE` (600 byte) header margin, needed to decode `discard`.
/// `discard` is clamped to `0..=MAX_DISCARD_LEVEL` like `LLImageJ2C::calcDataSize`.
pub fn bytes_for_discard(info: &J2kInfo, discard: u8) -> usize {
    calc_data_size_j2c(
        info.width as i64,
        info.height as i64,
        discard.min(MAX_DISCARD_LEVEL) as i64,
        DEFAULT_COMPRESSION_RATE,
    )
}

fn calc_data_size_j2c(w: i64, h: i64, discard_level: i64, rate: f32) -> usize {
    const PRECISION: i64 = 8;
    const MAX_COMPONENTS: i64 = 4;
    const HARD_CAP: i64 = 12;
    const BASE_LAYER_AREA: i64 = MAX_BLOCK_SIZE * MAX_BLOCK_SIZE;
    const BITS_PER_TILE: i64 = MAX_COMPONENTS * PRECISION;

    let discard_layers = (5 - discard_level).max(0);
    let rate64 = rate as f64;
    let scaled_bits = |layer_area: i64| -> i64 { ((layer_area * BITS_PER_TILE) as f64 * rate64).round() as i64 };

    let total_bits = if w <= 0 || h <= 0 {
        scaled_bits(BASE_LAYER_AREA)
    } else {
        let surface = w * h;
        let mut layer_area = BASE_LAYER_AREA;
        let mut nb_layers: i64 = 1;
        let mut total = scaled_bits(layer_area);
        while surface > layer_area && nb_layers < HARD_CAP {
            if nb_layers <= discard_layers {
                total += scaled_bits(layer_area);
            }
            nb_layers += 1;
            layer_area *= 4;
        }
        let max_dimension = w.max(h);
        let ratio = max_dimension as f32 / MAX_BLOCK_SIZE as f32;
        let dimension_layers = if ratio > 0.0 {
            ((ratio.log2().floor() as i64) + 1).max(1)
        } else {
            1
        };
        if dimension_layers > nb_layers {
            let mut extra = dimension_layers.min(HARD_CAP) - nb_layers;
            while extra > 0 {
                extra -= 1;
                if nb_layers <= discard_layers {
                    total += scaled_bits(layer_area);
                }
                nb_layers += 1;
                layer_area *= 4;
            }
        }
        total
    };
    let est = total_bits / 8 + HEADER_SIZE_ESTIMATE as i64;
    est.clamp(0, i32::MAX as i64) as usize
}

// ---------------------------------------------------------------------------
// OpenJPEG FFI wrapper
// ---------------------------------------------------------------------------

/// In-memory input stream state (port of `JPEG2KBase` + `opj_read/skip/seek`).
struct MemStream<'a> {
    data: &'a [u8],
    offset: usize,
}

unsafe extern "C" fn mem_read(buffer: *mut c_void, nb_bytes: usize, user: *mut c_void) -> usize {
    if user.is_null() || buffer.is_null() {
        return usize::MAX;
    }
    // SAFETY: `user` is the `MemStream` registered with `opj_stream_set_user_data`;
    // it lives on the stack of `decode_j2k` for longer than the stream and is only
    // accessed from this (single) decoding thread.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    if s.offset >= s.data.len() {
        s.offset = s.data.len();
        return usize::MAX; // (OPJ_SIZE_T)-1: end of stream
    }
    let to_read = nb_bytes.min(s.data.len() - s.offset);
    // SAFETY: OpenJPEG guarantees `buffer` is writable for `nb_bytes` bytes, and
    // `to_read <= nb_bytes`; the source range is in bounds of `s.data` by the check above.
    unsafe {
        ptr::copy_nonoverlapping(s.data.as_ptr().add(s.offset), buffer as *mut u8, to_read);
    }
    s.offset += to_read;
    to_read
}

unsafe extern "C" fn mem_skip(nb_bytes: i64, user: *mut c_void) -> i64 {
    if user.is_null() {
        return -1;
    }
    // SAFETY: see `mem_read`.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    let len = s.data.len() as i64;
    let new_offset = (s.offset as i64).saturating_add(nb_bytes);
    if new_offset < 0 || new_offset > len {
        s.offset = new_offset.clamp(0, len) as usize;
        return -1;
    }
    s.offset = new_offset as usize;
    nb_bytes
}

unsafe extern "C" fn mem_seek(offset: i64, user: *mut c_void) -> opj::OPJ_BOOL {
    if user.is_null() {
        return 0;
    }
    // SAFETY: see `mem_read`.
    let s = unsafe { &mut *(user as *mut MemStream<'_>) };
    if offset < 0 || offset > s.data.len() as i64 {
        return 0;
    }
    s.offset = offset as usize;
    1
}

fn log_opj_message(level: log::Level, msg: *const c_char) {
    if msg.is_null() || !log::log_enabled!(target: "aurora_assets::j2k", level) {
        return;
    }
    // SAFETY: OpenJPEG passes a valid NUL-terminated C string that outlives the callback.
    let text = unsafe { CStr::from_ptr(msg) };
    log::log!(target: "aurora_assets::j2k", level, "OpenJPEG: {}", text.to_string_lossy().trim_end());
}

unsafe extern "C" fn opj_error_cb(msg: *const c_char, _: *mut c_void) {
    log_opj_message(log::Level::Debug, msg);
}

unsafe extern "C" fn opj_warning_cb(msg: *const c_char, _: *mut c_void) {
    log_opj_message(log::Level::Trace, msg);
}

unsafe extern "C" fn opj_info_cb(msg: *const c_char, _: *mut c_void) {
    log_opj_message(log::Level::Trace, msg);
}

struct StreamGuard(*mut opj::opj_stream_t);
impl Drop for StreamGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from `opj_stream_create` and is destroyed exactly once.
            unsafe { opj::opj_stream_destroy(self.0) }
        }
    }
}

struct CodecGuard(*mut opj::opj_codec_t);
impl Drop for CodecGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from `opj_create_decompress` and is destroyed exactly once.
            unsafe { opj::opj_destroy_codec(self.0) }
        }
    }
}

struct ImageGuard(*mut opj::opj_image_t);
impl Drop for ImageGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the image was allocated by OpenJPEG (`opj_read_header`) and is
            // destroyed exactly once, after which nothing references it.
            unsafe { opj::opj_image_destroy(self.0) }
        }
    }
}

/// Decode a J2K codestream (or JP2 file) at the given discard level.
///
/// `discard` is used as OpenJPEG `cp_reduce` and is clamped to the number of
/// decomposition levels. Truncated codestreams (partial HTTP range fetches)
/// are decoded as far as possible (non-strict mode). Each call uses its own
/// codec instance, so this is safe to call concurrently.
pub fn decode_j2k(data: &[u8], discard: u8) -> Result<DecodedImage, AssetError> {
    let cs = locate_codestream(data).ok_or(AssetError::J2k("not a JPEG2000 codestream"))?;
    let info = parse_main_header(cs).ok_or(AssetError::Truncated("JPEG2000 main header"))?;
    if info.width > MAX_DIMENSION || info.height > MAX_DIMENSION || info.width as u64 * info.height as u64 > MAX_PIXELS {
        return Err(AssetError::TooLarge("JPEG2000 image dimensions"));
    }
    if u16::from(info.components) > MAX_HEADER_COMPONENTS {
        return Err(AssetError::Unsupported(format!("JPEG2000 with {} components", info.components)));
    }
    let reduce = discard.min(info.levels);

    let mut mem = MemStream { data: cs, offset: 0 };

    // SAFETY: plain FFI constructor; null is checked below.
    let stream = StreamGuard(unsafe { opj::opj_stream_create(cs.len().clamp(4096, 1 << 20), 1) });
    if stream.0.is_null() {
        return Err(AssetError::J2k("opj_stream_create failed"));
    }
    // SAFETY: `stream.0` is a valid input stream. The user data pointer refers to
    // `mem`, which is declared before `stream` and therefore dropped after it; no
    // free function is registered because `mem` is owned by this stack frame.
    unsafe {
        opj::opj_stream_set_user_data(stream.0, &mut mem as *mut MemStream<'_> as *mut c_void, None);
        opj::opj_stream_set_user_data_length(stream.0, cs.len() as u64);
        opj::opj_stream_set_read_function(stream.0, Some(mem_read));
        opj::opj_stream_set_skip_function(stream.0, Some(mem_skip));
        opj::opj_stream_set_seek_function(stream.0, Some(mem_seek));
    }

    // SAFETY: plain FFI constructor; null is checked below.
    let codec = CodecGuard(unsafe { opj::opj_create_decompress(opj::CODEC_FORMAT::OPJ_CODEC_J2K) });
    if codec.0.is_null() {
        return Err(AssetError::J2k("opj_create_decompress failed"));
    }

    // SAFETY: `opj_dparameters_t` is a plain C struct (integers and char arrays) for
    // which all-zero is a valid bit pattern; it is then initialised by OpenJPEG.
    let mut params: opj::opj_dparameters_t = unsafe { std::mem::zeroed() };
    // SAFETY: `params` is a valid, exclusively borrowed parameter struct.
    unsafe { opj::opj_set_default_decoder_parameters(&mut params) };
    params.cp_reduce = u32::from(reduce);

    // SAFETY: `codec.0` is a valid decompressor; the callbacks are `extern "C"`
    // functions that ignore their (null) client data.
    let setup_ok = unsafe {
        opj::opj_set_error_handler(codec.0, Some(opj_error_cb), ptr::null_mut());
        opj::opj_set_warning_handler(codec.0, Some(opj_warning_cb), ptr::null_mut());
        opj::opj_set_info_handler(codec.0, Some(opj_info_cb), ptr::null_mut());
        let ok = opj::opj_setup_decoder(codec.0, &mut params);
        // Enable decoding of partially loaded images, as LLImageJ2COJ does.
        opj::opj_decoder_set_strict_mode(codec.0, 0);
        ok
    };
    if setup_ok == 0 {
        return Err(AssetError::J2k("opj_setup_decoder failed"));
    }

    let mut image = ImageGuard(ptr::null_mut());
    // SAFETY: stream and codec are valid; `image.0` receives an OpenJPEG-owned image
    // (or stays null) which the guard destroys.
    let header_ok = unsafe { opj::opj_read_header(stream.0, codec.0, &mut image.0) };
    if header_ok == 0 || image.0.is_null() {
        return Err(AssetError::J2k("opj_read_header failed"));
    }

    // SAFETY: stream, codec and image are valid and belong to this call.
    let decoded = unsafe { opj::opj_decode(codec.0, stream.0, image.0) };
    // SAFETY: as above. The result is ignored like in LL: it fails for truncated
    // streams even though the decoded data is usable.
    unsafe { opj::opj_end_decompress(codec.0, stream.0) };
    if decoded == 0 {
        return Err(AssetError::J2k("opj_decode failed"));
    }

    // SAFETY: `image.0` is non-null and points to an image fully initialised by
    // `opj_read_header`/`opj_decode`; it is not mutated while borrowed.
    let img = unsafe { &*image.0 };
    convert_image(img)
}

/// Convert OpenJPEG component planes to interleaved 8-bit pixels.
fn convert_image(img: &opj::opj_image_t) -> Result<DecodedImage, AssetError> {
    if img.numcomps == 0 || img.comps.is_null() {
        return Err(AssetError::J2k("decoded image has no components"));
    }
    // SAFETY: OpenJPEG allocates `numcomps` contiguous `opj_image_comp_t` at `comps`.
    let comps = unsafe { std::slice::from_raw_parts(img.comps, img.numcomps as usize) };
    let used = &comps[..comps.len().min(4)];

    let mut width = 0u32;
    let mut height = 0u32;
    for c in used {
        if c.data.is_null() || c.w == 0 || c.h == 0 {
            return Err(AssetError::J2k("decoded component has no data"));
        }
        width = width.max(c.w);
        height = height.max(c.h);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(AssetError::TooLarge("decoded JPEG2000 dimensions"));
    }
    let n = used.len();
    let pixels = width as usize * height as usize;
    let mut out = vec![0u8; pixels * n];

    for (ci, c) in used.iter().enumerate() {
        let cw = c.w as usize;
        let ch = c.h as usize;
        // SAFETY: OpenJPEG allocates `w * h` samples for each decoded component
        // (dimensions already reduced by the resolution factor).
        let plane = unsafe { std::slice::from_raw_parts(c.data as *const i32, cw * ch) };
        let prec = c.prec.clamp(1, 31);
        let offset: i64 = if c.sgnd != 0 { 1i64 << (prec - 1) } else { 0 };
        let max_in = (1i64 << prec) - 1;
        let to8 = |v: i32| -> u8 {
            let v = (v as i64 + offset).clamp(0, max_in);
            let v = match prec.cmp(&8) {
                std::cmp::Ordering::Equal => v,
                std::cmp::Ordering::Greater => v >> (prec - 8),
                std::cmp::Ordering::Less => (v * 255 + max_in / 2) / max_in,
            };
            v.clamp(0, 255) as u8
        };
        let full = cw == width as usize && ch == height as usize;
        for y in 0..height as usize {
            let sy = if full { y } else { (y * ch / height as usize).min(ch - 1) };
            let src_row = &plane[sy * cw..sy * cw + cw];
            let dst_row = &mut out[y * width as usize * n..(y + 1) * width as usize * n];
            if full {
                for (x, &v) in src_row.iter().enumerate() {
                    dst_row[x * n + ci] = to8(v);
                }
            } else {
                for x in 0..width as usize {
                    let sx = (x * cw / width as usize).min(cw - 1);
                    dst_row[x * n + ci] = to8(src_row[sx]);
                }
            }
        }
    }

    Ok(DecodedImage {
        width,
        height,
        components: n as u8,
        data: out,
        discard: comps[0].factor.min(255) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecWriter {
        buf: Vec<u8>,
        pos: usize,
    }

    unsafe extern "C" fn w_write(buffer: *mut c_void, n: usize, user: *mut c_void) -> usize {
        // SAFETY: `user` is the VecWriter owned by `encode`, alive for the whole encode.
        let w = unsafe { &mut *(user as *mut VecWriter) };
        // SAFETY: OpenJPEG provides `n` readable bytes at `buffer`.
        let src = unsafe { std::slice::from_raw_parts(buffer as *const u8, n) };
        let end = w.pos + n;
        if w.buf.len() < end {
            w.buf.resize(end, 0);
        }
        w.buf[w.pos..end].copy_from_slice(src);
        w.pos = end;
        n
    }

    unsafe extern "C" fn w_skip(n: i64, user: *mut c_void) -> i64 {
        // SAFETY: see `w_write`.
        let w = unsafe { &mut *(user as *mut VecWriter) };
        let new = w.pos as i64 + n;
        if new < 0 {
            return -1;
        }
        w.pos = new as usize;
        if w.buf.len() < w.pos {
            w.buf.resize(w.pos, 0);
        }
        n
    }

    unsafe extern "C" fn w_seek(off: i64, user: *mut c_void) -> opj::OPJ_BOOL {
        // SAFETY: see `w_write`.
        let w = unsafe { &mut *(user as *mut VecWriter) };
        if off < 0 {
            return 0;
        }
        w.pos = off as usize;
        if w.buf.len() < w.pos {
            w.buf.resize(w.pos, 0);
        }
        1
    }

    /// Lossless J2K encode (test helper, mirrors JPEG2KEncode in llimagej2coj.cpp).
    fn encode(w: u32, h: u32, comps: u32, pixel: impl Fn(u32, u32, u32) -> u8) -> Vec<u8> {
        // SAFETY: test-only FFI usage; every pointer is checked and freed below.
        unsafe {
            let mut params: opj::opj_cparameters_t = std::mem::zeroed();
            opj::opj_set_default_encoder_parameters(&mut params);
            params.tcp_numlayers = 1;
            params.tcp_rates[0] = 0.0;
            params.cp_disto_alloc = 1;
            params.irreversible = 0;
            params.tcp_mct = if comps >= 3 { 1 } else { 0 };
            // Like JPEG2KEncode: small images need fewer resolutions.
            params.numresolution = params.numresolution.min(1 + w.min(h).ilog2() as i32);
            let mut cparms: Vec<opj::opj_image_cmptparm_t> = (0..comps)
                .map(|_| {
                    let mut p: opj::opj_image_cmptparm_t = std::mem::zeroed();
                    p.dx = 1;
                    p.dy = 1;
                    p.w = w;
                    p.h = h;
                    p.prec = 8;
                    p.sgnd = 0;
                    p
                })
                .collect();
            let image = opj::opj_image_create(comps, cparms.as_mut_ptr(), opj::COLOR_SPACE::OPJ_CLRSPC_SRGB);
            assert!(!image.is_null());
            (*image).x0 = 0;
            (*image).y0 = 0;
            (*image).x1 = w;
            (*image).y1 = h;
            for c in 0..comps {
                let comp = &*(*image).comps.add(c as usize);
                let plane = std::slice::from_raw_parts_mut(comp.data, (w * h) as usize);
                for y in 0..h {
                    for x in 0..w {
                        plane[(y * w + x) as usize] = pixel(x, y, c) as i32;
                    }
                }
            }
            let codec = opj::opj_create_compress(opj::CODEC_FORMAT::OPJ_CODEC_J2K);
            assert!(!codec.is_null());
            assert!(opj::opj_setup_encoder(codec, &mut params, image) != 0);
            let mut writer = VecWriter { buf: Vec::new(), pos: 0 };
            let stream = opj::opj_stream_create(1 << 16, 0);
            opj::opj_stream_set_user_data(stream, &mut writer as *mut VecWriter as *mut c_void, None);
            opj::opj_stream_set_write_function(stream, Some(w_write));
            opj::opj_stream_set_skip_function(stream, Some(w_skip));
            opj::opj_stream_set_seek_function(stream, Some(w_seek));
            assert!(opj::opj_start_compress(codec, image, stream) != 0);
            assert!(opj::opj_encode(codec, stream) != 0);
            assert!(opj::opj_end_compress(codec, stream) != 0);
            opj::opj_stream_destroy(stream);
            opj::opj_destroy_codec(codec);
            opj::opj_image_destroy(image);
            writer.buf
        }
    }

    fn pattern(x: u32, y: u32, c: u32) -> u8 {
        ((x * 4 + y * 3 + c * 50) & 0xFF) as u8
    }

    #[test]
    fn info_and_decode_rgb() {
        let enc = encode(64, 48, 3, pattern);
        let info = j2k_info(&enc).expect("info");
        assert_eq!(
            info,
            J2kInfo {
                width: 64,
                height: 48,
                components: 3,
                levels: 5
            }
        );

        let img = decode_j2k(&enc, 0).expect("decode d0");
        assert_eq!((img.width, img.height, img.components, img.discard), (64, 48, 3, 0));
        assert_eq!(img.data.len(), 64 * 48 * 3);
        for y in 0..48u32 {
            for x in 0..64u32 {
                for c in 0..3u32 {
                    assert_eq!(img.data[((y * 64 + x) * 3 + c) as usize], pattern(x, y, c));
                }
            }
        }

        let img1 = decode_j2k(&enc, 1).expect("decode d1");
        assert_eq!((img1.width, img1.height, img1.components, img1.discard), (32, 24, 3, 1));
        assert_eq!(img1.data.len(), 32 * 24 * 3);

        // Discard beyond the number of levels is clamped.
        let img9 = decode_j2k(&enc, 9).expect("decode d9");
        assert_eq!(img9.discard, 5);
        assert_eq!((img9.width, img9.height), (2, 2));
    }

    #[test]
    fn decode_gray_and_rgba() {
        let enc = encode(16, 16, 1, |x, y, _| (x * 16 + y) as u8);
        let img = decode_j2k(&enc, 0).expect("gray");
        assert_eq!(img.components, 1);
        assert_eq!(img.data[5 * 16 + 3], (3 * 16 + 5) as u8);

        let enc = encode(32, 32, 4, pattern);
        let img = decode_j2k(&enc, 0).expect("rgba");
        assert_eq!(img.components, 4);
        assert_eq!(img.data[(7 * 32 + 9) * 4 + 3], pattern(9, 7, 3));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode_j2k(&[], 0).is_err());
        assert!(decode_j2k(b"definitely not a jpeg2000 file", 0).is_err());
        let mut junk = vec![0xFF, 0x4F, 0xFF, 0x51];
        junk.extend((0..500u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
        assert!(decode_j2k(&junk, 0).is_err());
        assert!(j2k_info(&junk).is_none());
    }

    #[test]
    fn truncated_does_not_panic() {
        let enc = encode(128, 128, 3, pattern);
        for cut in [10, 60, 120, 200, enc.len() / 4, enc.len() / 2, enc.len() - 3] {
            let r = decode_j2k(&enc[..cut], 0);
            if let Ok(img) = r {
                assert_eq!(img.data.len(), (img.width * img.height * img.components as u32) as usize);
            }
        }
        // A cut that keeps the header must still decode (non-strict mode).
        let img = decode_j2k(&enc[..enc.len() / 2], 0).expect("partial decode");
        assert_eq!((img.width, img.height), (128, 128));
    }

    #[test]
    fn concurrent_decodes() {
        use rayon::prelude::*;
        let enc = encode(64, 64, 3, pattern);
        let ok = (0..32).into_par_iter().map(|i| decode_j2k(&enc, (i % 3) as u8).is_ok()).all(|b| b);
        assert!(ok);
    }

    #[test]
    fn data_size_estimate() {
        let info = J2kInfo {
            width: 1024,
            height: 1024,
            components: 4,
            levels: 5,
        };
        // Non-increasing with discard, always including the 600 byte header.
        let sizes: Vec<usize> = (0..=5).map(|d| bytes_for_discard(&info, d)).collect();
        for w in sizes.windows(2) {
            assert!(w[0] >= w[1]);
        }
        assert!(sizes[0] > sizes[2]);
        // discard 5 => only the base 64x64 layer: 64*64*32/8 bits / 8 + 600
        assert_eq!(sizes[5], 64 * 64 * 32 / 8 / 8 + 600);
        // Clamped like LLImageJ2C::calcDataSize.
        assert_eq!(bytes_for_discard(&info, 200), sizes[5]);
    }
}
