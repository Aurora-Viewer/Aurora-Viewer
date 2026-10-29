//! Local translation of chat messages: languages, detection, prompts, the
//! model catalogue and a worker thread around the model. No interface and
//! no protocol: the viewer's `translate` module glues it to the IMs.

pub mod detect;
pub mod lang;
pub mod prompt;

pub use detect::{Detected, detect, nothing_to_translate};
pub use lang::{LANGS, Lang, lang, parse_lang};
pub use prompt::{PromptKind, Rejected, clean_output, prompt};
