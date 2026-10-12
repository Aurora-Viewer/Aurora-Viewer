//! Reader of the hand-edited news files (`news/<language>.md` at the
//! repository root). One `## date — title` heading per item, followed by its
//! plain text and an optional link:
//!
//! ```text
//! ## 2026-10-08 — Title of the news item
//! One or more lines of summary text.
//! [Read more](https://example.org/optional-link)
//! ```
//!
//! The file is written by hand and downloaded: whatever is not understood is
//! skipped, never an error, and every item goes through `NewsItem::bounded`.
use super::{MAX_ITEMS, NewsItem};
use std::borrow::Cow;

pub fn parse(text: &str) -> Vec<NewsItem> {
    let text = without_comments(text.trim_start_matches('\u{feff}'));
    let mut items = Vec::new();
    let mut draft: Option<Draft> = None;
    for line in text.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("## ") {
            items.extend(draft.take().and_then(Draft::finish));
            if items.len() >= MAX_ITEMS {
                break;
            }
            draft = Some(Draft::new(heading));
        } else if is_decoration(line) {
            // The page title, sub-headings and rules only dress the file up on GitHub.
        } else if let Some(draft) = &mut draft {
            draft.line(line);
        }
    }
    items.extend(draft.and_then(Draft::finish));
    items.truncate(MAX_ITEMS);
    items
}

/// `YYYY-MM-DD` with a plausible month and day. Dates are only displayed and
/// compared as text, so the calendar is not checked further.
pub fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    let number = |from: usize, to: usize| {
        b[from..to]
            .iter()
            .try_fold(0u32, |n, c| c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0')))
    };
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && number(0, 4).is_some()
        && number(5, 7).is_some_and(|month| (1..=12).contains(&month))
        && number(8, 10).is_some_and(|day| (1..=31).contains(&day))
}

/// HTML comments hold the instructions for the editors, example item included.
fn without_comments(text: &str) -> Cow<'_, str> {
    if !text.contains("<!--") {
        return Cow::Borrowed(text);
    }
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        kept.push_str(&rest[..start]);
        // An unclosed comment hides the rest of the file, as on GitHub.
        rest = rest[start..].find("-->").map_or("", |end| &rest[start + end + 3..]);
    }
    kept.push_str(rest);
    Cow::Owned(kept)
}

/// Headings of another level (`#` to `######`, then a space) and rules
/// (`---`, `***`, `===`).
fn is_decoration(line: &str) -> bool {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    let heading = (1..=6).contains(&level) && (line.len() == level || line[level..].starts_with([' ', '\t']));
    let rule = line.len() >= 3 && line.chars().all(|c| matches!(c, '-' | '*' | '_' | '='));
    heading || rule
}

struct Draft {
    date: String,
    title: String,
    paragraphs: Vec<String>,
    /// An empty line was read: the next text starts a new paragraph.
    paragraph_break: bool,
    url: String,
}

impl Draft {
    fn new(heading: &str) -> Self {
        // "## Title ##" is a valid heading too; the "#" of "C#" belongs to the title.
        let unclosed = heading.trim_end_matches('#');
        let heading = if unclosed.is_empty() || unclosed.ends_with(char::is_whitespace) {
            unclosed
        } else {
            heading
        }
        .trim();
        let (date, title) = split_date(heading).unwrap_or(("", heading));
        Self {
            date: date.to_owned(),
            title: plain(title, &mut Vec::new()),
            paragraphs: Vec::new(),
            paragraph_break: true,
            url: String::new(),
        }
    }

    fn line(&mut self, line: &str) {
        if line.is_empty() {
            self.paragraph_break = true;
            return;
        }
        let mut targets = Vec::new();
        let text = if is_bare_url(line) {
            targets.push(line.trim_matches(['<', '>']));
            String::new()
        } else {
            plain(line, &mut targets)
        };
        // The first usable link wins; the item has a single click target.
        if self.url.is_empty()
            && let Some(url) = targets.into_iter().find(|url| super::is_https(url))
        {
            self.url = url.to_owned();
        }
        // A link alone on its line ("[Read more](…)") is the click target, not text.
        if text.is_empty() || is_lone_link(line) {
            return;
        }
        match self.paragraphs.last_mut() {
            Some(paragraph) if !self.paragraph_break => {
                paragraph.push(' ');
                paragraph.push_str(&text);
            }
            _ => self.paragraphs.push(text),
        }
        self.paragraph_break = false;
    }

    fn finish(self) -> Option<NewsItem> {
        NewsItem {
            date: self.date,
            title: self.title,
            summary: self.paragraphs.join("\n"),
            url: self.url,
        }
        .bounded()
    }
}

/// Splits `2026-10-08 — Title` (or `-`, `:`, `|`, or only a space after the date).
fn split_date(heading: &str) -> Option<(&str, &str)> {
    let date = heading.get(..10).filter(|date| is_date(date))?;
    let rest = &heading[10..];
    // "2026-10-081" or "2026-10-08th" is not a date followed by a title.
    if rest.starts_with(char::is_alphanumeric) {
        return None;
    }
    let rest = rest.trim_start();
    let title = rest.strip_prefix(['—', '–', '-', ':', '|']).unwrap_or(rest).trim_start();
    Some((date, title))
}

fn is_bare_url(line: &str) -> bool {
    let line = line.trim_matches(['<', '>']);
    line.contains("://") && !line.contains(char::is_whitespace) && !line.contains(['[', ']', '(', ')'])
}

fn is_lone_link(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(')') && line.matches("](").count() == 1
}

/// Text of a line: `[label](target)` becomes `label` and its target is
/// reported, images are dropped.
fn plain<'a>(line: &'a str, targets: &mut Vec<&'a str>) -> String {
    let mut text = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(middle) = rest.find("](") {
        let after = &rest[middle + 2..];
        let (Some(open), Some(close)) = (rest[..middle].rfind('['), after.find(')')) else {
            // Not a link: keep the characters as they are and look further.
            text.push_str(&rest[..middle + 2]);
            rest = after;
            continue;
        };
        let before = &rest[..open];
        if let Some(before) = before.strip_suffix('!') {
            text.push_str(before);
        } else {
            text.push_str(before);
            text.push_str(&rest[open + 1..middle]);
            // `(https://example.org "tooltip")`: the target ends at the first space.
            targets.extend(after[..close].split_whitespace().next());
        }
        rest = &after[close + 1..];
    }
    text.push_str(rest);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::news::{MAX_SUMMARY, MAX_TITLE, MAX_URL};

    fn item(date: &str, title: &str, summary: &str, url: &str) -> NewsItem {
        NewsItem {
            date: date.into(),
            title: title.into(),
            summary: summary.into(),
            url: url.into(),
        }
    }

    #[test]
    fn well_formed_file() {
        let text = "<!-- How to edit this file:\n## 2020-01-01 — Example, not a news item\n-->\n\n# Aurora Viewer news\n\n\
            ## 2026-10-08 — Second item\nFirst line,\nsecond line.\n\nSecond paragraph.\n[Read more](https://example.org/second)\n\n\
            ---\n\n## 2026-09-30 - First item\nOnly one line.\n";
        assert_eq!(
            parse(text),
            [
                item(
                    "2026-10-08",
                    "Second item",
                    "First line, second line.\nSecond paragraph.",
                    "https://example.org/second"
                ),
                item("2026-09-30", "First item", "Only one line.", ""),
            ]
        );
    }

    #[test]
    fn missing_date_keeps_the_whole_heading_as_title() {
        for (heading, date, title) in [
            ("Title without a date", "", "Title without a date"),
            ("08/10/2026 — French date", "", "08/10/2026 — French date"),
            ("2026-13-08 — Month 13", "", "2026-13-08 — Month 13"),
            ("2026-10-081 Not a date", "", "2026-10-081 Not a date"),
            ("2026-10-08: Colon", "2026-10-08", "Colon"),
            ("2026-10-08 Only a space", "2026-10-08", "Only a space"),
            ("2026-10-08 – Closed heading ##", "2026-10-08", "Closed heading"),
            ("2026-10-08 — Learning C#", "2026-10-08", "Learning C#"),
            ("2026-10-08 — -5 °C tonight", "2026-10-08", "-5 °C tonight"),
            ("Été — accents before the tenth byte", "", "Été — accents before the tenth byte"),
        ] {
            assert_eq!(
                parse(&format!("## {heading}\nText.")),
                [item(date, title, "Text.", "")],
                "{heading}"
            );
        }
        // A date alone has no title to show.
        assert!(parse("## 2026-10-08\nText.").is_empty());
        assert!(parse("## 2026-10-08 —\nText.").is_empty());
    }

    #[test]
    fn links() {
        let url = |body: &str| parse(&format!("## Title\n{body}")).pop().map(|item| (item.summary, item.url));
        let some = |summary: &str, url: &str| Some((summary.to_owned(), url.to_owned()));
        // Missing link.
        assert_eq!(url("Text."), some("Text.", ""));
        // A link alone on its line is not text.
        assert_eq!(
            url("Text.\n[Read more](https://example.org/a)"),
            some("Text.", "https://example.org/a")
        );
        assert_eq!(
            url("[Read more](https://example.org/a \"Tooltip\")"),
            some("", "https://example.org/a")
        );
        // Only https links are kept, and their line never shows.
        for target in [
            "http://example.org/a",
            "ftp://example.org/a",
            "file:///C:/a",
            "javascript:alert(1)",
            "https://",
            "https://user:secret@example.org/",
            "example.org",
            "",
        ] {
            assert_eq!(url(&format!("Text.\n[Read more]({target})")), some("Text.", ""), "{target}");
        }
        // The first usable link wins.
        assert_eq!(
            url("[Bad](http://example.org/a)\n[Good](https://example.org/b)\n[Late](https://example.org/c)"),
            some("", "https://example.org/b")
        );
        // Inline links keep their label; images are dropped.
        assert_eq!(
            url("See the [release notes](https://example.org/notes) ![logo](https://example.org/logo.png) today."),
            some("See the release notes today.", "https://example.org/notes")
        );
        // Bare addresses.
        assert_eq!(url("Text.\nhttps://example.org/a"), some("Text.", "https://example.org/a"));
        assert_eq!(url("Text.\n<https://example.org/a>"), some("Text.", "https://example.org/a"));
        assert_eq!(url("Text.\nhttp://example.org/a"), some("Text.", ""));
        // Brackets that are not links stay in the text.
        assert_eq!(url("Array[0](x and [done]"), some("Array[0](x and [done]", ""));
        // An over-long address is dropped rather than cut.
        let long = format!("https://example.org/{}", "a".repeat(MAX_URL));
        assert_eq!(url(&format!("Text.\n[Read more]({long})")), some("Text.", ""));
    }

    #[test]
    fn garbage_and_empty_input() {
        for text in [
            "",
            "\n\n",
            "\u{feff}",
            "Just a sentence.",
            "# Only a page title\nText.",
            "##No space after the hashes\nText.",
            "## \nText.",
            "<!-- ## 2026-10-08 — Hidden",
            "[](",
            "](",
            "\0\u{1}\u{2}## \u{3}",
            "{\"date\": \"2026-10-08\", \"title\": \"JSON\"}",
        ] {
            assert!(parse(text).is_empty(), "{text:?}");
        }
        // Binary noise must not panic either.
        let noise: String = (0u32..4000)
            .filter_map(|i| char::from_u32(i.wrapping_mul(2_654_435_761) % 0x3000))
            .collect();
        parse(&noise);
        parse(&format!("## {noise}\n{noise}\n[{noise}]({noise})"));
    }

    #[test]
    fn items_are_bounded() {
        let many: String = (0..200).map(|i| format!("## 2026-01-01 — Item {i}\nText {i}.\n")).collect();
        let items = parse(&many);
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(items[0].title, "Item 0");
        assert_eq!(items[MAX_ITEMS - 1].title, format!("Item {}", MAX_ITEMS - 1));

        let long = parse(&format!("## {}\n{}", "é".repeat(500), "ü ".repeat(2000)));
        assert_eq!(long[0].title.chars().count(), MAX_TITLE);
        assert!(long[0].title.ends_with('…'));
        assert_eq!(long[0].summary.chars().count(), MAX_SUMMARY);
        assert!(long[0].summary.ends_with('…'));

        // Control characters never reach the interface.
        assert_eq!(
            parse("## Ti\u{7}tle\u{1b}[31m\nTe\u{0}xt\u{8}."),
            [item("", "Title[31m", "Text.", "")]
        );
    }

    #[test]
    fn crlf_line_endings() {
        let unix = "\u{feff}<!-- Comment\non two lines -->\n## 2026-10-08 — Title\nLine one\nline two.\n\nParagraph.\n[Read more](https://example.org/a)\n## Other\n";
        let windows = unix.replace('\n', "\r\n");
        let items = parse(&windows);
        assert_eq!(items, parse(unix));
        assert_eq!(
            items,
            [
                item("2026-10-08", "Title", "Line one line two.\nParagraph.", "https://example.org/a"),
                item("", "Other", "", ""),
            ]
        );
    }

    #[test]
    fn dates() {
        for date in ["2026-10-08", "1999-01-31", "2026-12-01"] {
            assert!(is_date(date), "{date}");
        }
        for date in [
            "",
            "2026-10-8",
            "2026-00-08",
            "2026-10-00",
            "2026-10-32",
            "2026/10/08",
            "08-10-2026",
            "2026-10-08 ",
            "２０２６-10-08",
        ] {
            assert!(!is_date(date), "{date}");
        }
    }
}
