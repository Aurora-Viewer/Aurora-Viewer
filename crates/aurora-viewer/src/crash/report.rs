//! Build the public draft only from the recorded session. The recovery
//! process's version, language, graphics and uptime are not substitutes.

use crate::bug_report::{Field, Report};
use std::path::Path;

pub(super) fn draft(record: &super::Record, dir: &Path) -> Report {
    let reason = if dir.join("panic.txt").metadata().is_ok_and(|m| m.len() > 0) {
        "Rust panic"
    } else if dir.join("native.txt").metadata().is_ok_and(|m| m.len() > 0) {
        "Native exception"
    } else {
        "Unexpected shutdown"
    };
    let mut fields = vec![
        Field::new("bug-version", "Viewer version", &record.version, true),
        Field::new("bug-build", "Build profile", &record.profile, true),
        Field::new("bug-language", "Viewer language", &record.language, true),
        Field::new("crash-report-reason", "Crash type", reason, true),
    ];
    if !record.commit.is_empty() {
        fields.insert(1, Field::new("bug-commit", "Build commit", &record.commit, true));
    }
    if let Ok(code) = super::read_limited(&dir.join("exit-code.txt"), 32)
        && let Some(code) = code.trim().strip_prefix("0x").and_then(|hex| u32::from_str_radix(hex, 16).ok())
    {
        fields.push(Field::new(
            "crash-report-exit-code",
            "Process exit code",
            format!("0x{code:08X}"),
            true,
        ));
    }
    if let Ok(stopped) = super::read_limited(&dir.join("stopped-unix.txt"), 32)
        && let Ok(stopped) = stopped.trim().parse::<u64>()
        && let Some(duration) = stopped.checked_sub(record.started_unix)
    {
        fields.push(Field::new("bug-duration", "Session duration", format!("{duration} s"), true));
    }
    Report {
        title: aurora_i18n::tr!("crash-report-summary").into(),
        expected: aurora_i18n::tr!("crash-report-expected").into(),
        actual: aurora_i18n::tr!("crash-report-actual").into(),
        fields,
        // The user describes what happened, and opts into filtered diagnostics.
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_draft_keeps_original_session_and_excludes_unselected_data() {
        let dir = std::env::temp_dir().join(format!("aurora-crash-report-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let record = super::super::Record {
            schema: 1,
            pid: 123,
            process_started: 456,
            started_unix: 1000,
            version: "ORIGINAL_VERSION".into(),
            commit: "ORIGINAL_COMMIT".into(),
            profile: "release".into(),
            language: "fr".into(),
            font: "Inter".into(),
            font_scale: 1.0,
            theme: Default::default(),
            log_path: dir.join("DO_NOT_PUBLISH.log"),
            demo: true,
            os: String::new(),
            crash_reports: Default::default(),
        };
        std::fs::write(dir.join("exit-code.txt"), "0xC0000005").unwrap();
        std::fs::write(dir.join("stopped-unix.txt"), "1042").unwrap();
        std::fs::write(dir.join("native.txt"), "Native exception: PRIVATE_TRACE").unwrap();
        let mut report = draft(&record, &dir);
        assert!(report.ready());
        assert!(report.steps.is_empty());
        assert!(!report.include_excerpt);
        let body = report.markdown();
        for value in [
            "ORIGINAL_VERSION",
            "ORIGINAL_COMMIT",
            "release",
            "fr",
            "0xC0000005",
            "42 s",
            "Native exception",
        ] {
            assert!(body.contains(value));
        }
        assert!(!body.contains("PRIVATE_TRACE"));
        assert!(!body.contains("DO_NOT_PUBLISH"));
        for field in &mut report.fields {
            field.included = false;
        }
        let body = report.markdown();
        assert!(!body.contains("ORIGINAL_"));
        assert!(!body.contains("0xC0000005"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_crash_metadata_does_not_invent_exit_code_or_recovery_uptime() {
        let dir = std::env::temp_dir().join(format!("aurora-crash-report-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut record: super::super::Record = serde_json::from_str(&format!(
            r##"{{"schema":1,"pid":1,"process_started":0,"started_unix":0,"version":"old","profile":"debug","language":"en","font":"Inter","font_scale":1,"theme":{},"log_path":"old.log","demo":true}}"##,
            serde_json::to_string(&crate::theme::Theme::default()).unwrap()
        ))
        .unwrap();
        let report = draft(&record, &dir);
        assert!(
            report
                .fields
                .iter()
                .all(|f| !matches!(f.label, "bug-commit" | "bug-duration" | "crash-report-exit-code"))
        );
        std::fs::write(dir.join("exit-code.txt"), "0xPRIVATE_TEXT").unwrap();
        record.commit = "OLD_COMMIT".into();
        assert!(!draft(&record, &dir).markdown().contains("PRIVATE_TEXT"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
