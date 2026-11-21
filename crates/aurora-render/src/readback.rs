//! Alignment-independent decoding for bytes returned by mapped GPU buffers.
//!
//! The Legacy Viewer reads timer results from GL query objects into scalar
//! values (`indra/llrender/llglslshader.cpp`,
//! `LLGLSLShader::placeProfileQuery` / `readProfileQuery`); Aurora copies
//! mapped bytes into scalars instead.

/// Round a buffer allocation up so adjacent read-back allocations keep an
/// eight-byte boundary even when a preceding payload contains `u32` values.
pub const fn readback_size(size: u64) -> u64 {
    (size + 7) & !7
}

#[cfg(test)]
pub fn u32s(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|chunk| u32::from_le_bytes(*chunk)).collect()
}

#[cfg(test)]
pub fn u64s(bytes: &[u8]) -> Vec<u64> {
    bytes.as_chunks::<8>().0.iter().map(|chunk| u64::from_le_bytes(*chunk)).collect()
}

pub fn u32_array<const N: usize>(bytes: &[u8]) -> [u32; N] {
    std::array::from_fn(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("four bytes")))
}

pub fn u64_array<const N: usize>(bytes: &[u8]) -> [u64; N] {
    std::array::from_fn(|i| u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().expect("eight bytes")))
}

pub fn elapsed_gpu_ms(timestamps: &[u64], start: u32, end: u32, period_ns: f32) -> f32 {
    let (a, b) = (timestamps[start as usize], timestamps[end as usize]);
    (b.saturating_sub(a) as f64 * period_ns as f64 / 1e6) as f32
}

#[cfg(test)]
pub fn f32s(bytes: &[u8]) -> Vec<f32> {
    u32s(bytes).into_iter().map(f32::from_bits).collect()
}

#[cfg(test)]
pub fn f32x4s(bytes: &[u8]) -> Vec<[f32; 4]> {
    f32s(bytes).as_chunks::<4>().0.to_vec()
}

#[cfg(test)]
pub fn pods<T: bytemuck::Pod + Copy>(bytes: &[u8]) -> Vec<T> {
    bytes
        .chunks_exact(std::mem::size_of::<T>())
        .map(bytemuck::pod_read_unaligned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoders_accept_every_offset_modulo_eight() {
        let words = [0x0102_0304u32, 0xa0b0_c0d0, 0x7fc0_1234];
        let wide = [0x0102_0304_0506_0708u64, 0xa0b0_c0d0_e0f0_1234];
        let word_bytes: Vec<_> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
        let wide_bytes: Vec<_> = wide.iter().flat_map(|v| v.to_le_bytes()).collect();
        for offset in 0..8 {
            let mut storage = vec![0; offset];
            storage.extend_from_slice(&word_bytes);
            assert_eq!(u32s(&storage[offset..]), words);
            assert_eq!(u32_array::<3>(&storage[offset..]), words);
            assert_eq!(f32s(&storage[offset..])[2].to_bits(), words[2]);
            storage.clear();
            storage.resize(offset, 0);
            storage.extend_from_slice(&wide_bytes);
            assert_eq!(u64s(&storage[offset..]), wide);
            assert_eq!(u64_array::<2>(&storage[offset..]), wide);
        }
    }

    #[test]
    fn readback_allocations_are_eight_byte_aligned() {
        for size in 0..=32 {
            assert_eq!(readback_size(size) % 8, 0);
            assert!(readback_size(size) >= size);
            assert!(readback_size(size) - size < 8);
        }
    }

    #[test]
    fn every_map_read_buffer_descriptor_rounds_its_allocation() {
        fn visit(dir: &std::path::Path) {
            for entry in std::fs::read_dir(dir).expect("read renderer source directory") {
                let path = entry.expect("renderer source entry").path();
                if path.is_dir() {
                    visit(&path);
                } else if path.extension().is_some_and(|ext| ext == "rs") && path.file_name().is_some_and(|name| name != "readback.rs") {
                    let source = std::fs::read_to_string(&path).expect("read renderer source");
                    let mut rest = source.as_str();
                    while let Some(start) = rest.find("BufferDescriptor {") {
                        rest = &rest[start + "BufferDescriptor {".len()..];
                        let mut depth = 1usize;
                        let mut end = 0usize;
                        for (i, b) in rest.bytes().enumerate() {
                            match b {
                                b'{' => depth += 1,
                                b'}' => depth -= 1,
                                _ => {}
                            }
                            if depth == 0 {
                                end = i;
                                break;
                            }
                        }
                        if end == 0 {
                            break;
                        }
                        let descriptor = &rest[..end];
                        rest = &rest[end + 1..];
                        if descriptor.contains("BufferUsages::MAP_READ") {
                            let size = descriptor.lines().find(|line| line.trim_start().starts_with("size:"));
                            assert!(
                                size.is_some_and(|line| line.contains("readback_size(")) || descriptor.contains("readback_size("),
                                "MAP_READ allocation is not rounded in {}: {size:?}",
                                path.display()
                            );
                        }
                    }
                }
            }
        }

        visit(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
        // pick.rs shares a buffer factory between readback and non-readback
        // buffers, so it selects the rounded size from the usage flags.
        let pick = include_str!("pick.rs");
        assert!(pick.contains("usage.contains(wgpu::BufferUsages::MAP_READ)"));
        assert!(pick.contains("crate::readback::readback_size(BYTES)"));
    }

    #[test]
    fn unaligned_timestamp_bytes_preserve_gpu_elapsed_milliseconds() {
        let timestamps = [10_000u64, 35_000, 90_000];
        let bytes: Vec<_> = timestamps.iter().flat_map(|v| v.to_le_bytes()).collect();
        let expected = elapsed_gpu_ms(&timestamps, 0, 2, 1.25);
        for offset in 1..8 {
            let mut storage = vec![0; offset];
            storage.extend_from_slice(&bytes);
            // The timer decodes with `u64_array`, so test that path.
            let decoded = u64_array::<3>(&storage[offset..]);
            assert_eq!(elapsed_gpu_ms(&decoded, 0, 2, 1.25), expected);
        }
    }
}
