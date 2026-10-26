//! What the computer is (LLFloaterAbout::getInfo's CPU, memory, concurrency
//! and OS lines, indra/newview/llfloaterabout.cpp, originally LGPL 2.1),
//! read with sysinfo. Shared by About (`app/about.rs`) and the Dev
//! Toolbox's J4. sysinfo takes a while to read the system: the frame asks
//! a `Reading`, filled by a thread, and never waits for it.

use std::sync::{Arc, OnceLock};

/// The machine and the viewer's share of its memory.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    /// "Windows 11 Pro 26200 (x86_64)".
    pub os: String,
    pub cpu: String,
    /// Logical processors (the Legacy Viewer's "Concurrency").
    pub threads: usize,
    pub memory_mb: u64,
    /// What the viewer's process holds in memory now (the Legacy Viewer's
    /// "Used"), when the system tells.
    pub viewer_mb: Option<u64>,
}

impl Machine {
    /// Read the system now. Blocking: off the frame.
    pub fn read() -> Machine {
        use sysinfo::{CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let viewer_mb = sysinfo::get_current_pid().ok().and_then(|pid| {
            system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
            system.process(pid).map(|p| p.memory() / MIB)
        });
        Machine {
            os: format!(
                "{} ({})",
                System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()),
                std::env::consts::ARCH
            ),
            cpu: system.cpus().first().map_or("unknown", |c| c.brand().trim()).to_owned(),
            threads: system.cpus().len(),
            memory_mb: system.total_memory() / MIB,
            viewer_mb,
        }
    }

    /// The memory line, as the Legacy Viewer's AboutSystem writes it
    /// (English, for support): "32768 MB (viewer: 1204 MB)".
    pub fn memory_text(&self) -> String {
        match self.viewer_mb {
            Some(used) => format!("{} MB (viewer: {used} MB)", self.memory_mb),
            None => format!("{} MB", self.memory_mb),
        }
    }
}

const MIB: u64 = 1024 * 1024;

/// A `Machine` a thread is reading; empty until it is read.
#[derive(Clone, Debug, Default)]
pub struct Reading(Arc<OnceLock<Machine>>);

impl Reading {
    /// Start reading the system in the background.
    pub fn start() -> Reading {
        let reading = Reading::default();
        let fill = reading.0.clone();
        let spawned = std::thread::Builder::new().name("system-info".into()).spawn(move || {
            let _ = fill.set(Machine::read());
        });
        if let Err(e) = spawned {
            log::warn!("system information not read: {e}");
        }
        reading
    }

    /// A reading already done, for tests and the demo.
    #[cfg(test)]
    pub fn done(machine: Machine) -> Reading {
        let reading = Reading::default();
        let _ = reading.0.set(machine);
        reading
    }

    pub fn get(&self) -> Option<&Machine> {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_line_names_the_viewer_share_only_when_known() {
        let mut m = Machine {
            os: "Windows 11 (x86_64)".into(),
            cpu: "CPU".into(),
            threads: 16,
            memory_mb: 32768,
            viewer_mb: Some(1204),
        };
        assert_eq!(m.memory_text(), "32768 MB (viewer: 1204 MB)");
        m.viewer_mb = None;
        assert_eq!(m.memory_text(), "32768 MB");
    }

    #[test]
    fn a_reading_is_empty_until_filled() {
        assert!(Reading::default().get().is_none());
        let m = Machine {
            os: String::new(),
            cpu: String::new(),
            threads: 1,
            memory_mb: 1,
            viewer_mb: None,
        };
        assert_eq!(Reading::done(m.clone()).get(), Some(&m));
    }
}
