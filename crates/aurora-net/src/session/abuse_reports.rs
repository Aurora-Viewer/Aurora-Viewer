//! Abuse reports (LLFloaterReporter, indra/newview/llfloaterreporter.cpp,
//! originally LGPL 2.1): categories, the reported object's properties, and
//! the report itself, through the region's capabilities or, without them,
//! as a UserReport once the screenshot is uploaded (see `crate::abuse_report`).

use super::{Session, emit};
use crate::abuse_report::{self, AbuseReport, COMPLAINT_REPORT, COMPLAINT_REPORT_REQUEST};
use crate::types::{NetEvent, RegionHandle};
use aurora_msg::msgs::{RequestObjectPropertiesFamily, UserReport};
use aurora_msg::str_field;
use uuid::Uuid;

/// Longest text of a `Variable 1` field, its NUL included.
const VARIABLE_1: usize = 255;
/// Longest text of a `Variable 2` field, its NUL included.
const VARIABLE_2: usize = 65535;

impl Session<'_> {
    /// getObjectInfo: ask the object's own region (its properties come back
    /// as ObjectPropertiesFamily, with the request's flags).
    pub(super) fn request_report_object(&mut self, handle: RegionHandle, object: Uuid) {
        if let Some(addr) = self.sim_for_handle(handle) {
            let mut m = RequestObjectPropertiesFamily::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.object_data.request_flags = COMPLAINT_REPORT_REQUEST;
            m.object_data.object_id = object;
            self.send(addr, &m, true);
        }
    }

    /// requestAbuseCategoriesCoro: only when the region has the capability;
    /// the window keeps its own list otherwise.
    pub(super) fn request_abuse_categories(&self, language: &str) {
        let Some(cap) = self.main_cap("AbuseCategories") else {
            return;
        };
        let url = abuse_report::categories_url(&cap, language);
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match abuse_report::categories(&http, &url).await {
                Some(list) => {
                    let _ = events.send(NetEvent::AbuseCategories(list));
                }
                None => log::warn!("abuse report: no categories from the region"),
            }
        });
    }

    /// onClickSend: the capabilities when the region has one of them,
    /// otherwise the screenshot's upload then UserReport.
    pub(super) fn send_abuse_report(&mut self, mut report: AbuseReport) {
        // the screenshot's asset id is known before its upload (the texture
        // control holds it) and goes in the report
        let transaction = Uuid::new_v4();
        let screenshot_id = if report.screenshot.is_some() {
            abuse_report::asset_id(transaction, self.login.secure_session_id)
        } else {
            Uuid::nil()
        };
        let plain = self.main_cap("SendUserReport");
        let with_screenshot = self.main_cap("SendUserReportWithScreenshot");
        if plain.is_some() || with_screenshot.is_some() {
            let body = report.body(screenshot_id);
            let screenshot = report.screenshot.take();
            let http = self.sh.caps_http.clone();
            let events = self.sh.events.clone();
            tokio::spawn(async move {
                let result = abuse_report::send(&http, plain.as_deref(), with_screenshot.as_deref(), body, screenshot).await;
                if let Err(e) = &result {
                    log::warn!("abuse report: {e}");
                }
                let _ = events.send(NetEvent::AbuseReportSent(result));
            });
            return;
        }
        match report.screenshot.take() {
            // uploadImage, then uploadDoneCallback sends the report
            Some(data) => self.upload_report_screenshot(transaction, screenshot_id, data, report),
            None => {
                self.send_user_report(&report, Uuid::nil());
                emit(self.sh, NetEvent::AbuseReportSent(Ok(())));
            }
        }
    }

    /// sendReportViaLegacy.
    pub(super) fn send_user_report(&mut self, report: &AbuseReport, screenshot_id: Uuid) {
        let mut m = UserReport::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        let d = &mut m.report_data;
        d.report_type = COMPLAINT_REPORT;
        d.category = u8::try_from(report.category).unwrap_or(0);
        d.position = report.position;
        d.check_flags = 0;
        d.screenshot_id = screenshot_id;
        d.object_id = report.object_id;
        d.abuser_id = report.abuser_id;
        d.abuse_region_name = str_field("");
        d.abuse_region_id = Uuid::nil();
        d.summary = text_field(&report.summary, VARIABLE_1);
        d.details = text_field(&report.details, VARIABLE_2);
        d.version_string = text_field(&report.version_string, VARIABLE_1);
        self.send_main(&m, true);
    }
}

/// A NUL-terminated string field cut to `limit` bytes, on a character.
fn text_field(text: &str, limit: usize) -> Vec<u8> {
    let mut end = text.len().min(limit - 1);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    str_field(&text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_texts_are_cut_on_a_character_with_their_nul() {
        let text = "é".repeat(200);
        let field = text_field(&text, VARIABLE_1);
        assert!(field.len() <= VARIABLE_1);
        assert_eq!(field.last(), Some(&0));
        assert!(std::str::from_utf8(&field[..field.len() - 1]).is_ok());
        assert_eq!(text_field("spam", VARIABLE_1), b"spam\0");
    }
}
