//! One GPU device at a time across this crate's tests. The test harness
//! runs tests on parallel threads, and two tests creating and using a
//! device at once on the CI runner's software adapter ended the whole test
//! binary with an access violation (merge queue of #499, 2026-10-16). A
//! test that creates a device holds this lock until it ends.

use std::sync::{Mutex, MutexGuard};

static GPU: Mutex<()> = Mutex::new(());

/// The lock; a test that panicked while holding it does not block the
/// next ones.
pub fn lock() -> MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `fragment` WGSL with each `in.clip.w` read as WGSL defines it, 1/w, on
/// the adapter the test took. WGSL's fragment `@builtin(position).w` is
/// 1/w (as on Vulkan and Metal); D3D's `SV_Position.w` is w, and naga's
/// HLSL output passes it through. The viewer never runs on D3D
/// (`crate::BACKENDS`), but the default adapter is DX12 on the Windows CI
/// runner (WARP), so a test whose fragment depth reads `in.clip.w` gets w
/// there unless the read is inverted.
pub fn fragment_inv_w(fragment: &str, adapter: &wgpu::Adapter) -> String {
    assert!(fragment.contains("in.clip.w"), "no fragment 1/w read to adapt");
    if adapter.get_info().backend == wgpu::Backend::Dx12 {
        fragment.replace("in.clip.w", "(1.0/in.clip.w)")
    } else {
        fragment.to_owned()
    }
}
