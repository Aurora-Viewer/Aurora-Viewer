//! User-reviewed diagnostics. Inspired by Firestorm's handle_report_bug
//! (indra/newview/llviewermenu.cpp, originally LGPL 2.1). Unlike its direct
//! environment/location link, Aurora makes every collected field optional.
//! This model deliberately has no access to login, session or chat state.

pub mod files;
pub mod github;
pub mod privacy;

use std::fmt::Write;

#[derive(Clone, Debug)]
pub struct Field {
    pub label: &'static str,
    pub heading: &'static str,
    pub value: String,
    pub included: bool,
    /// Localized presentation of composite values; the export stays stable.
    pub details: Vec<(&'static str, String)>,
}

impl Field {
    pub fn new(label: &'static str, heading: &'static str, value: impl Into<String>, included: bool) -> Self {
        Self {
            label,
            heading,
            value: value.into(),
            included,
            details: Vec::new(),
        }
    }

    pub fn display_value(&self) -> String {
        if !self.details.is_empty() {
            return self
                .details
                .iter()
                .map(|(key, value)| {
                    let value = match value.as_str() {
                        "true" => aurora_i18n::tr!("common-yes"),
                        "false" => aurora_i18n::tr!("common-no"),
                        _ => value,
                    };
                    format!("{} : {value}", aurora_i18n::text(key))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if self.value == "Unavailable" {
            return aurora_i18n::tr!("bug-unavailable").into();
        }
        if self.label == "bug-build" && self.value == "debug" {
            return aurora_i18n::tr!("bug-build-development").into();
        }
        if self.label == "crash-report-reason" {
            return aurora_i18n::text(match self.value.as_str() {
                "Rust panic" => "crash-reason-rust",
                "Native exception" => "crash-reason-native",
                _ => "crash-reason-unknown",
            })
            .into();
        }
        self.value.clone()
    }
}

#[derive(Clone, Default)]
pub struct Report {
    pub title: String,
    pub steps: String,
    pub expected: String,
    pub actual: String,
    pub fields: Vec<Field>,
    pub excerpt: String,
    pub include_excerpt: bool,
}

impl Report {
    pub fn ready(&self) -> bool {
        !self.title.trim().is_empty() && !self.actual.trim().is_empty()
    }

    /// A single serializer feeds both the preview and the saved/public body.
    /// Excluded values never enter this string (not even as HTML comments).
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        for (heading, body) in [
            ("Steps to reproduce", self.steps.as_str()),
            ("Expected behavior", self.expected.as_str()),
            ("Actual behavior", self.actual.as_str()),
        ] {
            let _ = writeln!(out, "## {heading}\n\n{}\n", body.trim());
        }
        for field in self.fields.iter().filter(|f| f.included && !f.value.is_empty()) {
            let _ = writeln!(out, "## {}\n\n{}\n", field.heading, plain_markdown(&field.value));
        }
        if self.include_excerpt && !self.excerpt.is_empty() {
            let _ = writeln!(out, "## Filtered diagnostics\n\n```text\n{}\n```\n", self.excerpt);
        }
        out
    }
}

fn plain_markdown(value: &str) -> String {
    // Collected labels (e.g. an avatar/region name) are data, not markup,
    // mentions or links which GitHub should interpret as active content.
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('@', "&#64;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_can_be_excluded_independently() {
        let mut r = Report {
            fields: (0..12)
                .map(|i| Field::new("test", "Test", format!("PRIVATE_VALUE_{i}_END"), true))
                .collect(),
            excerpt: "PRIVATE_LOG".into(),
            include_excerpt: true,
            ..Default::default()
        };
        for i in 0..r.fields.len() {
            r.fields[i].included = false;
            let body = r.markdown();
            assert!(!body.contains(&format!("PRIVATE_VALUE_{i}_END")));
            for j in i + 1..r.fields.len() {
                assert!(body.contains(&format!("PRIVATE_VALUE_{j}_END")));
            }
        }
        r.include_excerpt = false;
        assert!(!r.markdown().contains("PRIVATE_"));
    }

    #[test]
    fn optional_diagnostics_start_unchecked_and_names_cannot_create_mentions() {
        let r = Report {
            fields: vec![Field::new("test", "Avatar", "<img src=x> @someone", true)],
            ..Default::default()
        };
        assert!(!r.include_excerpt);
        assert!(!r.markdown().contains("<img"));
        assert!(!r.markdown().contains("@someone"));
        assert!(!r.ready());
    }
}
