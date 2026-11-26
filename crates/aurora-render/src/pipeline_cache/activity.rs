//! What the render thread is doing when it has kept a frame long enough to
//! be sampled. A compiler worker making pipelines on the same device says
//! nothing about where the render thread is. Look up its exact thread ID.

use std::fmt;
use std::sync::Mutex;
use std::thread::{self, ThreadId};

const RENDER: &str = "create render pipeline";
const COMPUTE: &str = "create compute pipeline";
const WAIT: &str = "wait for pipeline worker";

/// The crash report is public. Only these renderer-owned descriptor labels
/// may cross that boundary, including when reading a session from disk.
const LABELS: &[&str] = &[
    "cs_mip",
    "cs_radiance",
    "cs_irradiance",
    "cs_irradiance_capture",
    "taa",
    "fsr autogen reactive",
    "fsr prepare inputs",
    "fsr farthest depth",
    "fsr shading change pyramid",
    "fsr downsample",
    "fsr shading change",
    "fsr prepare reactivity",
    "fsr luma instability",
    "fsr accumulate",
    "fsr rcas",
    "fsr display depth",
    "shadow merge: extract",
    "shadow merge: merge",
    "UI backdrop",
    "prefilter_depth0",
    "prefilter_depth",
    "main",
    "temporal",
    "denoise",
    "denoise_final",
    "denoise_bent",
    "main_bitmask",
    "denoise_il",
    "denoise_il_final",
    "contact shadows",
    "contact shadows filter",
    "contact shadows view ray",
    "contact shadows (variable reach)",
    "contact shadow pipelines",
    "stochastic total alpha",
    "stochastic depth",
    "stochastic accumulation",
    "stochastic glow",
    "oit stochastic resolve",
    "oit stochastic temporal blend",
    "oit stochastic smoothing",
    "oit stochastic tiles",
    "oit stochastic composite",
    "oit stochastic composite glow",
    "oit stochastic expand",
];

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PipelineWork {
    operation: &'static str,
    label: Option<&'static str>,
}

impl PipelineWork {
    fn new(operation: &'static str, label: Option<&str>) -> Self {
        Self {
            operation,
            label: label.and_then(|s| LABELS.iter().copied().find(|entry| *entry == s)),
        }
    }

    /// Read back a report field, rejecting unknown free text.
    pub fn from_report(s: &str) -> Option<Self> {
        let (op, label) = s.split_once(" / ").map_or((s, None), |(op, label)| (op, Some(label)));
        let operation = [RENDER, COMPUTE, WAIT].into_iter().find(|entry| *entry == op)?;
        let parsed = Self::new(operation, label);
        (parsed.label == label).then_some(parsed)
    }
}

impl fmt::Display for PipelineWork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.operation)?;
        if let Some(label) = self.label {
            write!(f, " / {label}")?;
        }
        Ok(())
    }
}

// A recovering device may leave its old render worker in the driver while
// the new one runs. Store them separately, with no lock held in the driver.
static ACTIVE: Mutex<Vec<(ThreadId, PipelineWork)>> = Mutex::new(Vec::new());

pub(crate) fn current(id: ThreadId) -> Option<PipelineWork> {
    ACTIVE
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(tid, _)| *tid == id)
        .map(|(_, work)| *work)
}

pub(super) struct Guard(ThreadId);

impl Guard {
    fn enter(operation: &'static str, label: Option<&str>) -> Option<Self> {
        let here = thread::current();
        if here.name() != Some("aurora-render") {
            return None;
        }
        let id = here.id();
        ACTIVE.lock().unwrap().push((id, PipelineWork::new(operation, label)));
        Some(Self(id))
    }

    pub(super) fn render(label: Option<&str>) -> Option<Self> {
        Self::enter(RENDER, label)
    }

    pub(super) fn compute(label: Option<&str>) -> Option<Self> {
        Self::enter(COMPUTE, label)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().unwrap();
        if let Some(index) = active.iter().rposition(|(id, _)| *id == self.0) {
            active.remove(index);
        }
    }
}

pub(crate) fn waiting_for<T>(label: &'static str, wait: impl FnOnce() -> T) -> T {
    let _waiting = Guard::enter(WAIT, Some(label));
    wait()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_sampling_is_per_render_thread_and_clears_on_unwind() {
        let (ready, started) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let render = thread::Builder::new()
            .name("aurora-render".into())
            .spawn(move || {
                waiting_for("contact shadow pipelines", || {
                    let _work = Guard::compute(Some("contact shadows (variable reach)"));
                    ready.send(()).unwrap();
                    wait.recv().unwrap();
                });
                assert!(current(thread::current().id()).is_none());
                let _ = std::panic::catch_unwind(|| {
                    let _work = Guard::render(Some("stochastic depth"));
                    panic!("test unwind");
                });
                assert!(current(thread::current().id()).is_none());
            })
            .unwrap();
        started.recv().unwrap();
        assert_eq!(
            current(render.thread().id()).unwrap().to_string(),
            "create compute pipeline / contact shadows (variable reach)"
        );
        thread::Builder::new()
            .name("aurora-render".into())
            .spawn(|| {
                let here = thread::current().id();
                assert!(
                    current(here).is_none(),
                    "a second renderer must not borrow the first worker's evidence"
                );
                let _work = Guard::render(Some("private asset label"));
                assert_eq!(current(here).unwrap().to_string(), "create render pipeline");
            })
            .unwrap()
            .join()
            .unwrap();
        let _other = Guard::compute(Some("contact shadows"));
        assert!(current(thread::current().id()).is_none());
        release.send(()).unwrap();
        render.join().unwrap();
    }

    #[test]
    fn report_fields_accept_only_known_labels_and_operations() {
        for op in [RENDER, COMPUTE, WAIT] {
            for label in [None, Some("stochastic depth")] {
                let known = PipelineWork::new(op, label);
                assert_eq!(PipelineWork::from_report(&known.to_string()), Some(known));
            }
        }
        assert!(PipelineWork::from_report("create compute pipeline / private name").is_none());
        assert!(PipelineWork::from_report("invented operation").is_none());
    }
}
