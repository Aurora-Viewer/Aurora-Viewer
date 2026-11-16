//! The self-test's report: one line per case. It names commands, points and
//! the test's own folders, never an asset id nor a resident.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Skipped(String),
    /// Fails for a reason already filed: the issue is named.
    Known(&'static str),
}

#[derive(Debug, Clone)]
pub struct Line {
    pub verdict: Verdict,
    pub id: &'static str,
    pub title: &'static str,
    /// The commands issued, as written in the case (`{ch}`, `{seat}`… are
    /// left unfilled: no id nor channel in the report).
    pub commands: Vec<String>,
    pub expected: String,
    pub observed: String,
}

impl Line {
    pub fn text(&self) -> String {
        let verdict = match &self.verdict {
            Verdict::Pass => "PASS".to_owned(),
            Verdict::Fail => "FAIL".to_owned(),
            Verdict::Skipped(why) => format!("SKIPPED ({why})"),
            Verdict::Known(issue) => format!("KNOWN ({issue})"),
        };
        let mut text = format!("{verdict}  {} {}", self.id, self.title);
        if !self.commands.is_empty() {
            text.push_str(" | commands: ");
            text.push_str(&self.commands.join(" ; "));
        }
        if !self.expected.is_empty() {
            text.push_str(" | expected: ");
            text.push_str(&self.expected);
        }
        if !self.observed.is_empty() {
            text.push_str(" | observed: ");
            text.push_str(&self.observed);
        }
        text
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub lines: Vec<Line>,
    /// Why the run ended early, what could not be cleaned up.
    pub notes: Vec<String>,
    pub controller: &'static str,
    pub phase2: bool,
}

impl Report {
    pub fn count(&self, like: fn(&Verdict) -> bool) -> usize {
        self.lines.iter().filter(|l| like(&l.verdict)).count()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} PASS, {} FAIL, {} SKIPPED, {} KNOWN",
            self.count(|v| matches!(v, Verdict::Pass)),
            self.count(|v| matches!(v, Verdict::Fail)),
            self.count(|v| matches!(v, Verdict::Skipped(_))),
            self.count(|v| matches!(v, Verdict::Known(_))),
        )
    }

    /// The body of the report (the Dev Toolbox adds the build header).
    pub fn text(&self) -> String {
        let mut text = format!(
            "RLVa self-test: {}\ncontroller: {}; in-world cases (phase 2): {}\n",
            self.summary(),
            self.controller,
            if self.phase2 { "chosen" } else { "not chosen" }
        );
        for note in &self.notes {
            text.push_str("NOTE  ");
            text.push_str(note);
            text.push('\n');
        }
        for line in &self.lines {
            text.push_str(&line.text());
            text.push('\n');
        }
        text
    }
}
