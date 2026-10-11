//! Terrain `LayerData` decoding: bit-packed DCT patches.
//!
//! Port of `llmessage/patch_code.cpp`, `patch_idct.cpp` and
//! `llcommon/llbitpack.h` (originally LGPL 2.1).

pub const LAND_LAYER_CODE: u8 = b'L';
pub const LAND_LAYER_CODE_LARGE: u8 = b'M';
pub const WIND_LAYER_CODE: u8 = b'7';
const END_OF_PATCHES: u8 = 97;
const OO_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// One decoded terrain patch (`size * size` heights, row-major, y rows).
#[derive(Debug, Clone)]
pub struct TerrainPatch {
    pub x: u32,
    pub y: u32,
    pub size: u32,
    pub heights: Vec<f32>,
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    load: u8,
    load_size: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            load: 0,
            load_size: 0,
        }
    }

    /// Mirrors `LLBitPack::bitUnpack` into a little-endian integer: bits are
    /// read MSB-first, 8 at a time, each chunk filling successive bytes.
    fn unpack(&mut self, mut count: u32) -> Option<u32> {
        let mut result: u32 = 0;
        let mut byte_index = 0;
        while count > 0 {
            let dsize = count.min(8);
            count -= dsize;
            let mut byte: u8 = 0;
            for _ in 0..dsize {
                if self.load_size == 0 {
                    self.load = *self.data.get(self.pos)?;
                    self.pos += 1;
                    self.load_size = 8;
                }
                byte = (byte << 1) | (self.load >> 7);
                self.load_size -= 1;
                self.load <<= 1;
            }
            if byte_index < 4 {
                result |= (byte as u32) << (8 * byte_index);
            }
            byte_index += 1;
        }
        Some(result)
    }
}

struct Tables {
    size: usize,
    dequant: Vec<f32>,
    icos: Vec<f32>,
    decopy: Vec<usize>,
}

impl Tables {
    fn new(size: usize) -> Tables {
        let mut dequant = vec![0.0; size * size];
        for j in 0..size {
            for i in 0..size {
                dequant[j * size + i] = 1.0 + 2.0 * (i + j) as f32;
            }
        }
        let mut icos = vec![0.0; size * size];
        let oosob = std::f32::consts::PI * 0.5 / size as f32;
        for u in 0..size {
            for n in 0..size {
                icos[u * size + n] = ((2.0 * n as f32 + 1.0) * u as f32 * oosob).cos();
            }
        }
        // Zig-zag copy matrix
        let mut decopy = vec![0usize; size * size];
        let (mut i, mut j) = (0usize, 0usize);
        let mut count = 0usize;
        let mut diag = false;
        let mut right = true;
        while i < size && j < size {
            decopy[j * size + i] = count;
            count += 1;
            if !diag {
                if right {
                    if i < size - 1 {
                        i += 1;
                    } else {
                        j += 1;
                    }
                    right = false;
                    diag = true;
                } else {
                    if j < size - 1 {
                        j += 1;
                    } else {
                        i += 1;
                    }
                    right = true;
                    diag = true;
                }
            } else if right {
                i += 1;
                j = j.wrapping_sub(1);
                if i == size - 1 || j == 0 {
                    diag = false;
                }
            } else {
                i = i.wrapping_sub(1);
                j += 1;
                if i == 0 || j == size - 1 {
                    diag = false;
                }
            }
        }
        Tables {
            size,
            dequant,
            icos,
            decopy,
        }
    }

    fn idct(&self, block: &mut [f32]) {
        let size = self.size;
        let mut temp = vec![0.0f32; size * size];
        // columns
        for column in 0..size {
            for n in 0..size {
                let mut total = OO_SQRT2 * block[column];
                for u in 1..size {
                    total += block[u * size + column] * self.icos[u * size + n];
                }
                temp[size * n + column] = total;
            }
        }
        // lines
        let oosob = 2.0 / size as f32;
        for line in 0..size {
            let ls = line * size;
            for n in 0..size {
                let mut total = OO_SQRT2 * temp[ls];
                for u in 1..size {
                    total += temp[ls + u] * self.icos[u * size + n];
                }
                block[ls + n] = total * oosob;
            }
        }
    }
}

/// Decode a land `LayerData` blob into patches.
pub fn decode_land_layer(layer_type: u8, data: &[u8]) -> Vec<TerrainPatch> {
    let large = match layer_type {
        LAND_LAYER_CODE => false,
        LAND_LAYER_CODE_LARGE => true,
        _ => return Vec::new(),
    };
    decode_layer(data, large, usize::MAX)
}

/// LLWind::decompress: two 16×16 DCT patches, X then Y. Wind always
/// uses the small patch header, including Firestorm's alternate layer '9'.
pub fn decode_wind_layer(layer_type: u8, data: &[u8]) -> Option<WindGrid> {
    if !matches!(layer_type, WIND_LAYER_CODE | b'9') {
        return None;
    }
    let patches = decode_layer(data, false, 2);
    if patches.len() != 2 || patches.iter().any(|p| p.size != 16) {
        return None;
    }
    Some(WindGrid {
        velocities: std::array::from_fn(|i| glam::Vec3::new(patches[0].heights[i], patches[1].heights[i], 0.0)),
    })
}

/// LLWind::getVelocity (indra/newview/llwind.cpp, LGPL 2.1): bilinear
/// sampling inside the lattice, nearest sample on its last row/column,
/// wrapped positive coordinates and WIND_SCALE_HACK = 2.
#[derive(Debug, Clone)]
pub struct WindGrid {
    pub velocities: [glam::Vec3; 256],
}

impl Default for WindGrid {
    fn default() -> Self {
        Self {
            velocities: [glam::Vec3::ZERO; 256],
        }
    }
}

impl WindGrid {
    pub fn velocity(&self, position: glam::Vec3, width: f32) -> glam::Vec3 {
        if !position.is_finite() || !width.is_finite() || width <= 0.0 {
            return glam::Vec3::ZERO;
        }
        let x = position.x.max(0.0).rem_euclid(width) * (16.0 / width);
        let y = position.y.max(0.0).rem_euclid(width) * (16.0 / width);
        let (i, j) = ((x as usize).min(15), (y as usize).min(15));
        let k = i + j * 16;
        let v = if i < 15 && j < 15 {
            self.velocities[k]
                .lerp(self.velocities[k + 1], x - i as f32)
                .lerp(self.velocities[k + 16].lerp(self.velocities[k + 17], x - i as f32), y - j as f32)
        } else {
            self.velocities[k]
        };
        v * 2.0
    }
}

fn decode_layer(data: &[u8], large: bool, max_patches: usize) -> Vec<TerrainPatch> {
    let mut out = Vec::new();
    let mut br = BitReader::new(data);
    // Group header
    let Some(_stride) = br.unpack(16) else {
        return out;
    };
    let Some(patch_size) = br.unpack(8) else {
        return out;
    };
    let Some(_ltype) = br.unpack(8) else {
        return out;
    };
    let size = patch_size as usize;
    if size != 16 && size != 32 {
        return out;
    }
    let tables = Tables::new(size);
    let mut coeffs = vec![0i32; size * size];

    while let Some(quant_wbits) = br.unpack(8) {
        if out.len() == max_patches {
            break;
        }
        let quant_wbits = quant_wbits as u8;
        if quant_wbits == END_OF_PATCHES {
            break;
        }
        let Some(dc_bits) = br.unpack(32) else {
            break;
        };
        let dc_offset = f32::from_bits(dc_bits);
        let Some(range) = br.unpack(16) else {
            break;
        };
        let Some(patchids) = br.unpack(if large { 32 } else { 10 }) else {
            break;
        };
        let (px, py) = if large {
            (patchids >> 16, patchids & 0xFFFF)
        } else {
            (patchids >> 5, patchids & 0x1F)
        };
        let wbits = (quant_wbits & 0x0f) as u32 + 2;

        // decode_patch
        let mut ok = true;
        let mut i = 0;
        while i < size * size {
            let Some(b) = br.unpack(1) else {
                ok = false;
                break;
            };
            if b != 0 {
                let Some(b2) = br.unpack(1) else {
                    ok = false;
                    break;
                };
                if b2 != 0 {
                    let Some(neg) = br.unpack(1) else {
                        ok = false;
                        break;
                    };
                    let Some(v) = br.unpack(wbits) else {
                        ok = false;
                        break;
                    };
                    coeffs[i] = if neg != 0 { -(v as i32) } else { v as i32 };
                } else {
                    for c in coeffs[i..].iter_mut() {
                        *c = 0;
                    }
                    break;
                }
            } else {
                coeffs[i] = 0;
            }
            i += 1;
        }
        if !ok {
            break;
        }

        // decompress_patch
        let prequant = (quant_wbits >> 4) as i32 + 2;
        let quantize = 1i32 << prequant;
        let ooq = 1.0 / quantize as f32;
        let mult = ooq * range as f32;
        let addval = mult * (1i32 << (prequant - 1)) as f32 + dc_offset;
        let mut block = vec![0.0f32; size * size];
        for k in 0..size * size {
            block[k] = coeffs[tables.decopy[k]] as f32 * tables.dequant[k];
        }
        tables.idct(&mut block);
        let heights: Vec<f32> = block.iter().map(|v| v * mult + addval).collect();
        if heights.iter().all(|h| h.is_finite()) {
            out.push(TerrainPatch {
                x: px,
                y: py,
                size: size as u32,
                heights,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_reader_matches_ll() {
        // 0b1010_1010, 0b1100_0000
        let d = [0xAA, 0xC0];
        let mut br = BitReader::new(&d);
        assert_eq!(br.unpack(1), Some(1));
        assert_eq!(br.unpack(1), Some(0));
        // 10 bits: first 8 -> low byte, next 2 -> high byte
        let v = br.unpack(10).unwrap();
        assert_eq!(v & 0xFF, 0b1010_1011);
        assert_eq!(v >> 8, 0b00);
    }

    #[test]
    fn zigzag_is_permutation() {
        let t = Tables::new(16);
        let mut seen = vec![false; 256];
        for &v in &t.decopy {
            seen[v] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn flat_patch_decodes() {
        // Build a stream: group header (stride 264, size 16, type L), one patch
        // with only DC zero coefficients (immediate EOB), then END.
        let mut bits: Vec<u8> = Vec::new();
        let mut push = |v: u32, n: u32| {
            // inverse of BitReader::unpack
            let mut rem = n;
            let mut idx = 0;
            while rem > 0 {
                let d = rem.min(8);
                rem -= d;
                let byte = (v >> (8 * idx)) & 0xFF;
                for b in (0..d).rev() {
                    bits.push(((byte >> b) & 1) as u8);
                }
                idx += 1;
            }
        };
        push(264, 16);
        push(16, 8);
        push(b'L' as u32, 8);
        push(0x11, 8); // quant_wbits
        push(20.0f32.to_bits(), 32);
        push(0, 16); // range
        push((3 << 5) | 4, 10);
        push(1, 1);
        push(0, 1); // EOB
        push(END_OF_PATCHES as u32, 8);
        let mut bytes = vec![0u8; bits.len().div_ceil(8)];
        for (i, b) in bits.iter().enumerate() {
            bytes[i / 8] |= b << (7 - (i % 8));
        }
        let p = decode_land_layer(b'L', &bytes);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].x, p[0].y), (3, 4));
        assert!(p[0].heights.iter().all(|h| (h - 20.0).abs() < 1e-3));
    }

    #[test]
    fn wind_decodes_both_components_and_rejects_partial_packets() {
        let mut bits = Vec::new();
        let mut push = |v: u32, n: u32| {
            for byte in 0..n.div_ceil(8) {
                let count = (n - byte * 8).min(8);
                for bit in (0..count).rev() {
                    bits.push(((v >> (byte * 8 + bit)) & 1) as u8);
                }
            }
        };
        push(16, 16);
        push(16, 8);
        push(WIND_LAYER_CODE as u32, 8);
        for v in [3.0f32, -2.0] {
            push(0x11, 8);
            push(v.to_bits(), 32);
            push(0, 16);
            push(0, 10);
            push(1, 1);
            push(0, 1);
        }
        push(END_OF_PATCHES as u32, 8);
        let mut bytes = vec![0u8; bits.len().div_ceil(8)];
        for (i, bit) in bits.iter().enumerate() {
            bytes[i / 8] |= bit << (7 - i % 8);
        }
        let grid = decode_wind_layer(WIND_LAYER_CODE, &bytes).expect("two wind patches");
        assert_eq!(
            grid.velocity(glam::Vec3::new(120.0, 88.0, 20.0), 256.0),
            glam::Vec3::new(6.0, -4.0, 0.0)
        );
        for len in 0..bytes.len() - 2 {
            assert!(decode_wind_layer(WIND_LAYER_CODE, &bytes[..len]).is_none());
        }
        assert!(decode_wind_layer(LAND_LAYER_CODE, &bytes).is_none());
        let grid = WindGrid {
            velocities: std::array::from_fn(|i| glam::Vec3::new((i % 16) as f32, (i / 16) as f32, 0.0)),
        };
        assert_eq!(
            grid.velocity(glam::Vec3::new(8.0, 24.0, 0.0), 256.0),
            glam::Vec3::new(1.0, 3.0, 0.0)
        );
        assert_eq!(
            grid.velocity(glam::Vec3::new(248.0, 248.0, 0.0), 256.0),
            glam::Vec3::new(30.0, 30.0, 0.0)
        );
        assert_eq!(
            grid.velocity(glam::Vec3::new(264.0, 24.0, 0.0), 256.0),
            glam::Vec3::new(1.0, 3.0, 0.0)
        );
    }
}
