//! Native file selection for a browser's `pick_file` request. Follows
//! LLViewerMediaImpl::handleMediaEvent / LLMediaFilePicker
//! (indra/newview/llviewermedia.cpp, llviewermenufile.cpp): all file types,
//! single or multiple selection, and an empty list on cancellation.

use crate::message::PluginMessage;
use aurora_llsd::Llsd;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct FilePicker {
    result: Receiver<Vec<PathBuf>>,
}

impl FilePicker {
    pub(super) fn open(multiple: bool) -> std::io::Result<Self> {
        Self::spawn(multiple, |multiple| {
            let dialog = rfd::FileDialog::new();
            if multiple {
                dialog.pick_files()
            } else {
                dialog.pick_file().map(|file| vec![file])
            }
        })
    }

    fn spawn(multiple: bool, choose: impl FnOnce(bool) -> Option<Vec<PathBuf>> + Send + 'static) -> std::io::Result<Self> {
        let (tx, result) = mpsc::channel();
        std::thread::Builder::new().name("media file picker".into()).spawn(move || {
            // A receiver belongs to one plugin instance. Closing that browser
            // discards the result; it can never reach a replacement browser.
            let _ = tx.send(choose(multiple).unwrap_or_default());
        })?;
        Ok(Self { result })
    }

    pub(super) fn poll(&self) -> Option<PluginMessage> {
        match self.result.try_recv() {
            Ok(files) => Some(response(files)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(response(Vec::new())),
        }
    }
}

pub(super) fn response(files: Vec<PathBuf>) -> PluginMessage {
    let files = files
        .into_iter()
        .filter_map(|file| match file.into_os_string().into_string() {
            Ok(file) => Some(Llsd::String(file)),
            Err(_) => {
                // The plugin protocol carries UTF-8. Never substitute a path
                // that could name a different local file.
                log::warn!("media: selected file path is not UTF-8");
                None
            }
        })
        .collect();
    PluginMessage::new(crate::media::CLASS_MEDIA, "pick_file_response")
        .with("file_list", Llsd::Array(files))
        .with("blocking_response", true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(picker: &FilePicker) -> PluginMessage {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(message) = picker.poll() {
                // Exercise the same LLSD wire format that the plugin reads.
                return PluginMessage::parse(message.generate().as_bytes()).expect("response");
            }
            assert!(Instant::now() < deadline, "file picker worker did not answer");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn media_file_picker_waits_without_cancelling_and_returns_unicode_paths() {
        for multiple in [false, true] {
            let (tx, rx) = mpsc::channel();
            let files = if multiple {
                vec![
                    PathBuf::from("/local/\u{e9}t\u{e9}.png"),
                    PathBuf::from("/local/\u{5199}\u{771f}.jpg"),
                ]
            } else {
                vec![PathBuf::from("/local/\u{e9}t\u{e9}.png")]
            };
            let expected = files.clone();
            let picker = FilePicker::spawn(multiple, move |requested_multiple| {
                assert_eq!(requested_multiple, multiple);
                rx.recv().expect("choose files")
            })
            .expect("worker");
            assert!(picker.poll().is_none(), "an open dialog must not send cancellation");
            tx.send(Some(files)).expect("finish selection");
            let message = wait(&picker);
            assert_eq!(message.class, "media");
            assert_eq!(message.name, "pick_file_response");
            assert!(message.boolean("blocking_response"));
            let paths: Vec<_> = message.get("file_list").as_array().iter().map(Llsd::to_string_value).collect();
            assert_eq!(paths, expected.iter().map(|p| p.to_str().expect("UTF-8")).collect::<Vec<_>>());
        }
    }

    #[test]
    fn media_file_picker_cancellation_and_failed_worker_unblock_the_browser() {
        let picker = FilePicker::spawn(false, |_| None).expect("worker");
        let cancelled = wait(&picker);
        assert!(cancelled.boolean("blocking_response"));
        assert_eq!(cancelled.get("file_list"), &Llsd::new_array());

        let (tx, result) = mpsc::channel();
        let picker = FilePicker { result };
        drop(tx);
        let failed = wait(&picker);
        assert!(failed.boolean("blocking_response"));
        assert_eq!(failed.get("file_list"), &Llsd::new_array());
    }

    #[test]
    fn media_file_picker_closed_browser_cannot_deliver_to_another_instance() {
        let (old_tx, result) = mpsc::channel();
        let old = FilePicker { result };
        drop(old);
        let current = FilePicker::spawn(false, |_| Some(vec![PathBuf::from("/local/current.txt")])).expect("worker");
        assert!(old_tx.send(vec![PathBuf::from("/local/old.txt")]).is_err());
        assert_eq!(wait(&current).get("file_list").as_array(), &[Llsd::from("/local/current.txt")]);
    }
}
