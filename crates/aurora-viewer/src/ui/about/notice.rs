//! The Licences tab reads NOTICE.md, built into the viewer, so that the
//! window and the repository never disagree (the Legacy Viewer's licenses
//! tab reads its licenses.txt the same way). Only what that file uses is
//! read: `##` headings, paragraphs, `-` and `*` items and tables, whose header and
//! rule rows are left out; links keep their text, bold and code marks go.

/// NOTICE.md at the repository root.
const NOTICE: &str = include_str!("../../../../../NOTICE.md");

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(String),
    Paragraph(String),
    Item(String),
    /// A table row: its first cell names the work, its last one the licence.
    Row {
        work: String,
        licence: String,
    },
}

/// NOTICE.md, read once.
pub fn blocks() -> &'static [Block] {
    static BLOCKS: std::sync::OnceLock<Vec<Block>> = std::sync::OnceLock::new();
    BLOCKS.get_or_init(|| parse(NOTICE))
}

pub fn parse(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    // the lines of the paragraph or item being read (true for an item):
    // an item's indented lines continue it
    let mut pending: Option<(bool, Vec<&str>)> = None;
    // a table's first row is its header
    let mut in_table = false;
    let flush = |pending: &mut Option<(bool, Vec<&str>)>, out: &mut Vec<Block>| {
        if let Some((item, lines)) = pending.take() {
            let text = inline(&lines.join(" "));
            out.push(if item { Block::Item(text) } else { Block::Paragraph(text) });
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(cells) = line.strip_prefix('|') {
            flush(&mut pending, &mut out);
            let cells: Vec<&str> = cells.trim_end_matches('|').split('|').map(str::trim).collect();
            let rule = cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')));
            if !in_table {
                in_table = true;
            } else if !rule && let (Some(first), Some(last)) = (cells.first(), cells.last()) {
                out.push(Block::Row {
                    work: inline(first),
                    licence: inline(last),
                });
            }
            continue;
        }
        in_table = false;
        if line.is_empty() || line.starts_with("# ") {
            // a blank line ends a block; the file's own title is left out
            // (the tab has its own)
            flush(&mut pending, &mut out);
        } else if let Some(heading) = line.strip_prefix("## ") {
            flush(&mut pending, &mut out);
            out.push(Block::Heading(inline(heading)));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush(&mut pending, &mut out);
            pending = Some((true, vec![item]));
        } else {
            pending.get_or_insert_with(|| (false, Vec::new())).1.push(line);
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Plain text of a Markdown line: `[text](url)` gives its text, `<url>`
/// its url; bold marks and backticks go (a lone `*` stays: `vcruntime140*.dll`).
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '['
            && let Some(close) = rest.find("](")
            && let Some(end) = rest[close..].find(')')
        {
            out.push_str(&rest[1..close]);
            rest = &rest[close + end + 1..];
            continue;
        }
        if c == '<'
            && let Some(end) = rest.find('>')
            && rest[1..end].starts_with("http")
        {
            out.push_str(&rest[1..end]);
            rest = &rest[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("**") {
            rest = after;
            continue;
        }
        if c != '`' {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_headings_paragraphs_items_and_table_rows() {
        let text = "# Notices\n\nFree **software**, see [LICENSE](LICENSE).\nSecond line.\n\n## Assets\n\n| Asset | Location | License |\n|---|---|---|\n| Icons — `a` | `x/` | MIT ([LICENSE](y)) |\n| `vc*.dll` | | MIT |\n\n- Viewer — <https://example.org>\n* Redistributions must\n  retain the notice.\n";
        assert_eq!(
            parse(text),
            vec![
                Block::Paragraph("Free software, see LICENSE. Second line.".into()),
                Block::Heading("Assets".into()),
                Block::Row {
                    work: "Icons — a".into(),
                    licence: "MIT (LICENSE)".into()
                },
                Block::Row {
                    work: "vc*.dll".into(),
                    licence: "MIT".into()
                },
                Block::Item("Viewer — https://example.org".into()),
                Block::Item("Redistributions must retain the notice.".into()),
            ]
        );
    }

    #[test]
    fn the_built_in_notice_names_the_licence_and_every_bundled_work() {
        let blocks = blocks();
        assert!(matches!(&blocks[0], Block::Paragraph(p) if p.contains("GNU General Public License")));
        let rows = blocks.iter().filter(|b| matches!(b, Block::Row { .. })).count();
        let table_lines = NOTICE.lines().filter(|l| l.trim_start().starts_with('|')).count();
        let tables = NOTICE.lines().filter(|l| l.trim_start().starts_with("|---")).count();
        // each table loses its header and its rule
        assert_eq!(rows, table_lines - 2 * tables);
        assert!(blocks.iter().all(|b| match b {
            Block::Heading(t) | Block::Paragraph(t) | Block::Item(t) => !t.contains("](") && !t.contains("**"),
            Block::Row { work, licence } => !work.is_empty() && !licence.is_empty() && !licence.contains("]("),
        }));
    }
}
