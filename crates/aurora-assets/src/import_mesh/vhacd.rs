//! Bounded C ABI around V-HACD 4.1. Native calls own no Rust memory after
//! compute returns; its synchronous worker joins every task before returning.

use super::{
    AssetError,
    process::{Hull, PhysicsOptions},
};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

unsafe extern "C" {
    fn aurora_vhacd_compute(
        points: *const f32,
        count: u32,
        triangles: *const u32,
        triangle_count: u32,
        resolution: u32,
        hulls: u32,
        vertices: u32,
        tolerance: f64,
        fill: u32,
        callback: extern "C" fn(*mut c_void, f64) -> bool,
        context: *mut c_void,
    ) -> *mut c_void;
    fn aurora_vhacd_free(result: *mut c_void);
    fn aurora_vhacd_count(result: *const c_void) -> u32;
    fn aurora_vhacd_hull(
        result: *const c_void,
        index: u32,
        points: *mut f32,
        point_count: *mut u32,
        triangles: *mut u32,
        triangle_count: *mut u32,
    ) -> bool;
}
struct Result(NonNull<c_void>);
impl Drop for Result {
    fn drop(&mut self) {
        // SAFETY: this is the unique handle returned by compute; Drop runs once.
        unsafe {
            aurora_vhacd_free(self.0.as_ptr());
        }
    }
}
struct Progress<'a> {
    cancel: &'a AtomicBool,
    percent: &'a AtomicU32,
}
extern "C" fn progress(context: *mut c_void, percent: f64) -> bool {
    // SAFETY: compute receives this stack context and joins its synchronous
    // tasks before returning. Shared access only touches atomic fields.
    let context = unsafe { &*(context.cast::<Progress<'_>>()) };
    if percent.is_finite() {
        context.percent.store(percent.clamp(0.0, 100.0) as u32, Ordering::Relaxed);
    }
    context.cancel.load(Ordering::Relaxed)
}
pub(super) fn analyze(
    points: &[[f32; 3]],
    triangles: &[[u32; 3]],
    options: PhysicsOptions,
    cancel: &AtomicBool,
    percent: &AtomicU32,
) -> std::result::Result<Vec<Hull>, AssetError> {
    let mut context = Progress { cancel, percent };
    // SAFETY: arrays have contiguous C-compatible scalars; all counts and
    // indices were checked by geometry(). Both buffers and context outlive
    // the synchronous call. C++ catches exceptions at this ABI boundary.
    let result = unsafe {
        aurora_vhacd_compute(
            points.as_ptr().cast(),
            points.len() as u32,
            triangles.as_ptr().cast(),
            triangles.len() as u32,
            options.resolution,
            options.max_hulls,
            options.max_vertices,
            f64::from(options.tolerance),
            options.fill_mode,
            progress,
            (&mut context as *mut Progress<'_>).cast(),
        )
    };
    let result = NonNull::new(result).map(Result);
    if cancel.load(Ordering::Relaxed) {
        return Err(AssetError::invalid("cancelled"));
    }
    let result = result.ok_or_else(|| AssetError::invalid("physics analysis failed"))?;
    // SAFETY: result remains alive until every hull has been copied.
    let count = unsafe { aurora_vhacd_count(result.0.as_ptr()) };
    if !(1..=options.max_hulls).contains(&count) {
        return Err(AssetError::invalid("physics hull count"));
    }
    let mut hulls = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut vertices = 0;
        let mut faces = 0;
        // SAFETY: null buffers request counts only; output pointers are valid.
        if !unsafe {
            aurora_vhacd_hull(
                result.0.as_ptr(),
                index,
                std::ptr::null_mut(),
                &mut vertices,
                std::ptr::null_mut(),
                &mut faces,
            )
        } || !(3..=options.max_vertices).contains(&vertices)
            || faces == 0
            || faces > 4096
        {
            return Err(AssetError::invalid("physics hull geometry"));
        }
        let mut hull = Hull {
            vertices: vec![[0.0; 3]; vertices as usize],
            indices: vec![[0; 3]; faces as usize],
        };
        // SAFETY: capacities match the queried counts and the C++ bridge
        // checks them again before writing. Scalars use the same ABI layout.
        if !unsafe {
            aurora_vhacd_hull(
                result.0.as_ptr(),
                index,
                hull.vertices.as_mut_ptr().cast(),
                &mut vertices,
                hull.indices.as_mut_ptr().cast(),
                &mut faces,
            )
        } || hull.vertices.iter().flatten().any(|v| !v.is_finite())
            || hull.indices.iter().flatten().any(|i| *i >= vertices)
        {
            return Err(AssetError::invalid("physics hull data"));
        }
        hulls.push(hull);
    }
    Ok(hulls)
}
