//! Metal keeps its ordinary device path; the report parser remains portable.
use super::{Boundary, Report};
use std::sync::Arc;
#[derive(Debug)]
pub(crate) struct Recorder;
pub(crate) struct Frame;
impl Recorder {
    pub(crate) fn frame(self: &Arc<Self>, _frame: u64) -> Option<Frame> {
        None
    }
    pub(crate) fn lost(self: &Arc<Self>, _confirmed: bool) {}
}
impl Frame {
    pub(crate) fn mark(&self, _encoder: &mut wgpu::CommandEncoder, _boundary: Boundary) {}
    pub(crate) fn finish(&self, _encoder: &mut wgpu::CommandEncoder) {}
    pub(crate) fn take_encoders(&self, tail: wgpu::CommandEncoder) -> Vec<wgpu::CommandEncoder> {
        vec![tail]
    }
    pub(crate) fn submitting(&mut self) {}
    pub(crate) fn completed(self) {}
}
pub fn first_loss() -> Option<Report> {
    None
}
pub fn set_sink(_sink: impl Fn(&Report, &[u8]) + Send + Sync + 'static) {}
