//! `dedicated.rs` where there is no Vulkan (macOS, on Metal): every texture
//! page comes from wgpu's allocator, which makes one MTLTexture per page and
//! frees it with the page, so the fragmentation `dedicated.rs` avoids does
//! not arise.

/// Bytes held by pages made here: none.
pub fn bytes() -> u64 {
    0
}

/// Never made here: the caller falls back to wgpu's allocator.
pub fn texture(_: &wgpu::Device, _: &wgpu::TextureDescriptor) -> Option<Result<wgpu::Texture, String>> {
    None
}
