//! Local translation of chat messages: languages, detection, prompts, the
//! model catalogue and a worker thread around the model. No interface and
//! no protocol: the viewer's `translate` module glues it to the IMs.

pub mod detect;
pub mod fake;
pub mod lang;
pub mod model;
pub mod prompt;
pub mod worker;

pub use detect::{Detected, detect, nothing_to_translate};
pub use fake::FakeBackend;
pub use lang::{LANGS, Lang, lang, parse_lang};
pub use model::{CATALOGUE, DEFAULT_MODEL, Download, DownloadError, DownloadState, ModelEntry, entry, is_ready, model_path};
pub use prompt::{PromptKind, Rejected, clean_output, prompt};
pub use worker::{Backend, BackendInfo, Direction, Done, Job, Status, TranslateError, Translator};
