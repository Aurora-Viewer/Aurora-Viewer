//! Local translation of chat messages: languages, detection, prompts, the
//! model catalogue and a worker thread around the model. No interface and
//! no protocol: the viewer's `translate` module glues it to the IMs.

pub mod detect;
pub mod fake;
pub mod lang;
#[cfg(feature = "llama")]
mod llama;
pub mod model;
pub mod prompt;
pub mod worker;

pub use detect::{Detected, detect, detect_between, nothing_to_translate};
pub use fake::FakeBackend;
pub use lang::{LANGS, Lang, lang, parse_lang};
pub use model::{CATALOGUE, DEFAULT_MODEL, Download, DownloadError, DownloadState, ModelEntry, entry, is_ready, model_path};
pub use prompt::{PromptKind, Rejected, clean_output, prompt};
pub use worker::{Backend, BackendInfo, Direction, Done, Job, Status, TranslateError, Translator};

/// The backend of a downloaded model: llama.cpp when the viewer is built
/// with it, otherwise one that answers `NotBuilt`.
pub fn backend_for(model_path: std::path::PathBuf) -> Box<dyn Backend> {
    #[cfg(feature = "llama")]
    {
        Box::new(llama::LlamaCpp::new(model_path))
    }
    #[cfg(not(feature = "llama"))]
    {
        let _ = model_path;
        Box::new(NotBuilt)
    }
}

/// Whether this build carries a model runtime.
pub const BUILT: bool = cfg!(feature = "llama");

#[cfg(not(feature = "llama"))]
struct NotBuilt;

#[cfg(not(feature = "llama"))]
impl Backend for NotBuilt {
    fn load(&mut self) -> Result<BackendInfo, TranslateError> {
        Err(TranslateError::NotBuilt)
    }
    fn generate(&mut self, _: &str, _: usize) -> Result<String, TranslateError> {
        Err(TranslateError::NotBuilt)
    }
    fn unload(&mut self) {}
}
