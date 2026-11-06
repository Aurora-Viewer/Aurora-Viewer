//! Bounded, best-effort Vulkan execution breadcrumbs. A boundary is evidence
//! of progress, never evidence that the following pass caused a device loss.
//! The Legacy Viewer uses OpenGL and has no equivalent Vulkan recorder.
mod model;
pub(crate) use model::Boundary;
pub use model::Report;
#[cfg(not(target_os = "macos"))]
use model::*;
#[cfg(not(target_os = "macos"))]
mod recorder;
#[cfg(target_os = "macos")]
#[path = "none.rs"]
mod recorder;
pub(crate) use recorder::Recorder;
pub use recorder::{first_loss, set_sink};
#[cfg(not(target_os = "macos"))]
pub(crate) mod vulkan;
