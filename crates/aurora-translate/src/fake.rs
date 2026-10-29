//! A backend without a model, for the tests and the demo: it answers
//! "[<target code>] <text>".

use crate::worker::{Backend, BackendInfo, TranslateError};
use std::time::Duration;

pub struct FakeBackend {
    panic: bool,
    fail: bool,
    delay: Duration,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self {
            panic: false,
            fail: false,
            delay: Duration::ZERO,
        }
    }

    /// Every translation fails.
    pub fn failing() -> Self {
        Self { fail: true, ..Self::new() }
    }

    /// The thread dies at the first translation (a crash in the runtime).
    pub fn panicking() -> Self {
        Self {
            panic: true,
            ..Self::new()
        }
    }

    /// Every translation takes `delay`, like a real model.
    pub fn slow(delay: Duration) -> Self {
        Self { delay, ..Self::new() }
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for FakeBackend {
    fn load(&mut self) -> Result<BackendInfo, TranslateError> {
        Ok(BackendInfo { on_gpu: false })
    }

    fn generate(&mut self, prompt: &str, _max_tokens: usize) -> Result<String, TranslateError> {
        std::thread::sleep(self.delay);
        assert!(!self.panic, "fake backend crash");
        if self.fail {
            return Err(TranslateError::Generate("fake failure".into()));
        }
        // "... to Spanish (es) translator. ...\n\n\n<text><end_of_turn>..."
        let to = prompt
            .split_once(") translator.")
            .and_then(|(head, _)| head.rsplit_once('('))
            .map_or("??", |(_, code)| code);
        let text = prompt
            .split_once("\n\n\n")
            .and_then(|(_, rest)| rest.split_once("<end_of_turn>"))
            .map_or("", |(text, _)| text);
        Ok(format!("[{to}] {text}"))
    }

    fn unload(&mut self) {}
}
