//! RlvSettings::updateLoginLastLocation and LLStartUp's start SLURL override
//! (rlvcommon.cpp, llstartup.cpp, LGPL 2.1). Only avatar UUIDs and next-login
//! flags persist. Ordered, coalesced writes run off the frame thread; disable
//! clears this policy too. Session restrictions are never serialized.
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::{collections::BTreeMap, path::PathBuf, thread::JoinHandle};
use uuid::Uuid;

type Flags = BTreeMap<Uuid, bool>;

struct Writer {
    sender: Option<Sender<Flags>>,
    discard: Receiver<Flags>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    fn start(path: PathBuf) -> Option<Self> {
        let (sender, writes) = crossbeam_channel::bounded::<Flags>(1);
        let discard = writes.clone();
        let worker = std::thread::Builder::new().name("aurora-rlva-login".into()).spawn(move || {
            while let Ok(mut flags) = writes.recv() {
                while let Ok(newer) = writes.try_recv() {
                    flags = newer;
                }
                let save = || -> std::io::Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = serde_json::to_vec(&flags).map_err(std::io::Error::other)?;
                    crate::atomic_file::write(&path, &bytes)
                };
                if let Err(error) = save() {
                    log::warn!("Cannot save RLVa next-login state: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Some(Self {
                sender: Some(sender),
                discard,
                worker: Some(worker),
            }),
            Err(error) => {
                log::warn!("Cannot start RLVa next-login storage: {error}");
                None
            }
        }
    }

    fn save(&self, mut flags: Flags) {
        let Some(sender) = &self.sender else {
            return;
        };
        loop {
            match sender.try_send(flags) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(latest)) => {
                    flags = latest;
                    let _ = self.discard.try_recv();
                }
            }
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub struct State {
    flags: Flags,
    path: Option<PathBuf>,
    writer: Option<Writer>,
}

impl State {
    /// Read once before the first frame, alongside ordinary settings. Only
    /// a small UUID/boolean catalogue is read; no account names or credentials.
    pub fn load(path: PathBuf) -> Self {
        let flags = match std::fs::read(&path) {
            Ok(bytes) => {
                crate::disk_io::read(crate::disk_io::Origin::Other, bytes.len());
                if bytes.len() <= 1024 * 1024 {
                    serde_json::from_slice(&bytes).unwrap_or_default()
                } else {
                    Flags::new()
                }
            }
            Err(_) => Flags::new(),
        };
        Self {
            flags,
            path: Some(path),
            writer: None,
        }
    }

    pub fn requires_last(&self, enabled: bool, agent: Option<Uuid>) -> bool {
        enabled && agent.and_then(|agent| self.flags.get(&agent)).copied().unwrap_or(true)
    }

    pub fn update(&mut self, enabled: bool, agent: Uuid, ready: bool, locked: bool) {
        let changed = if !enabled {
            let changed = !self.flags.is_empty();
            self.flags.clear();
            changed
        } else if ready && !agent.is_nil() {
            self.flags.insert(agent, locked) != Some(locked)
        } else {
            false
        };
        if !changed {
            return;
        }
        if self.writer.is_none()
            && let Some(path) = &self.path
        {
            self.writer = Writer::start(path.clone());
        }
        if let Some(writer) = &self.writer {
            writer.save(self.flags.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rlva_next_login_policy_keeps_native_initial_last_location_and_account_isolation() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut state = State::default();
        assert!(!state.requires_last(false, None));
        assert!(state.requires_last(true, None), "the native per-account flag defaults to true");
        assert!(state.requires_last(true, Some(a)));
        state.update(true, a, true, false);
        assert!(!state.requires_last(true, Some(a)));
        assert!(state.requires_last(true, Some(b)));
        state.update(true, a, true, true);
        assert!(state.requires_last(true, Some(a)));
        state.update(true, a, false, false);
        assert!(state.requires_last(true, Some(a)), "incomplete crossings cannot clear a lock");
        state.update(false, Uuid::nil(), false, false);
        assert!(state.flags.is_empty());
        assert!(!state.requires_last(false, Some(a)));
    }

    #[test]
    fn rlva_login_state_coalesces_writes_and_persists_the_final_disable_without_frame_thread_io() {
        let directory = std::env::temp_dir().join(format!("aurora-rlva-login-{}", Uuid::new_v4()));
        let path = directory.join("login.json");
        let a = Uuid::from_u128(1);
        {
            let mut state = State::load(path.clone());
            for locked in [true, false, true, false, true] {
                state.update(true, a, true, locked);
            }
        }
        {
            let mut restored = State::load(path.clone());
            assert!(restored.requires_last(true, Some(a)));
            assert_eq!(restored.flags.len(), 1);
            restored.update(false, Uuid::nil(), false, false);
        }
        let restored = State::load(path.clone());
        assert!(restored.flags.is_empty(), "shutdown flushes the latest cleared snapshot");
        drop(restored);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
