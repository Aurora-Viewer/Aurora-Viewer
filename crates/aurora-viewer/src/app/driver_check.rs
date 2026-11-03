//! The graphics driver's check when the renderer starts (`gpu_driver.rs`):
//! one log line, and the login screen's warning unless the user dismissed
//! it for this driver.

use super::*;
use crate::gpu_driver::{self, Input, Notice, Reason};

impl App {
    pub(super) fn driver_notice(&self, info: &aurora_render::GpuInfo) -> Option<Notice> {
        let input = gpu_driver::demo_input().unwrap_or_else(|| Input {
            vendor_id: info.vendor_id,
            gpu: info.name.clone(),
            driver: info.driver.clone(),
            linux: cfg!(target_os = "linux"),
            installed: gpu_driver::installed(&info.name),
            today: gpu_driver::today(),
        });
        let installed = input
            .installed
            .as_ref()
            .map(|i| format!(", installed {}-{:02}-{:02} as {}", i.date.0, i.date.1, i.date.2, i.version))
            .unwrap_or_default();
        let notice = gpu_driver::check(&input);
        match notice.as_ref().map(|n| &n.reason) {
            None => log::info!("graphics driver: {}{installed}: recent enough", input.driver),
            Some(Reason::Version { minimum, .. }) => {
                log::warn!(
                    "graphics driver: {}{installed}: out of date, {minimum} or newer recommended",
                    input.driver
                )
            }
            Some(Reason::Age { months, .. }) => {
                log::warn!(
                    "graphics driver: {}{installed}: out of date, installed {months} months ago",
                    input.driver
                )
            }
        }
        notice.filter(|n| n.key != self.settings.gpu_driver_dismissed)
    }
}
