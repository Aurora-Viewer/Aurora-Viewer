//! LSL script files in the importer: a `.lsl` file becomes a script item in
//! the agent's inventory, compiled for Mono and not running.
//!
//! The Legacy Viewer has no bulk script upload. This is an Aurora extension
//! built from its parts: the `.lsl` file filter (`LLFilePicker::FFLOAD_SCRIPT`,
//! indra/llui/llfilepicker.cpp), the reading of a script file
//! (`LLScriptEdCore::onBtnLoadFromFile` / `loadScriptFromFile`,
//! indra/newview/llpreviewscript.cpp) and the upload of its text
//! (`LLScriptAssetUpload`, indra/newview/llviewerassetupload.cpp), reached
//! through `NetCommand::CreateScriptItem` (new item, then UpdateScriptAgent).
//!
//! A script costs L$0 and never goes through NewFileAgentInventory, so it
//! has no upload `Kind` and no price: `Source::kind()` is `None` for it.
//! Scripts upload one at a time, like every other step of the queue, and each
//! is matched to its inventory item by the creation callback inside the
//! network layer, so scripts that share a name cannot be mixed up.

use super::*;
use std::{fs::File, io::Read, path::Path};

/// The grid's source limit, shared with the script editor.
pub const MAX_SOURCE: usize = aurora_net::scripts::MAX_SOURCE;

/// The inventory folder type of scripts (FT_LSL_TEXT), the default destination.
pub const FOLDER_TYPE: i32 = 10;

/// Lines of the source shown in the inspector.
pub const PREVIEW_LINES: usize = 60;

/// The name of an item whose file name leaves nothing usable (the Legacy
/// Viewer's untranslated default, as for a new script).
const DEFAULT_NAME: &str = "New Script";

/// A loaded script, shared cheaply with the upload step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// UTF-8, without byte order mark, with `\n` line ends.
    pub text: Arc<str>,
    pub lines: usize,
}

impl Draft {
    pub fn new(text: String) -> Self {
        let lines = line_count(&text);
        Self { text: text.into(), lines }
    }

    pub fn bytes(&self) -> usize {
        self.text.len()
    }

    /// The first `PREVIEW_LINES` lines, and whether more follow.
    pub fn head(&self) -> (&str, bool) {
        head(&self.text, PREVIEW_LINES)
    }
}

/// Why a script file is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Unreadable,
    NotUtf8,
    Empty,
    TooLarge,
}

impl Problem {
    pub fn message(self) -> &'static str {
        match self {
            Self::Unreadable => aurora_i18n::tr!("import-script-unreadable"),
            Self::NotUtf8 => aurora_i18n::tr!("import-script-not-utf8"),
            Self::Empty => aurora_i18n::tr!("import-script-empty"),
            Self::TooLarge => aurora_i18n::tr!("script-source-too-large"),
        }
    }
}

/// Whether `path` names a script (`.lsl`, in any case).
pub fn is_script(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lsl"))
}

/// Reads a script file, never more than the grid accepts plus a byte order
/// mark and one byte to tell that the file is too large.
pub fn read(path: &Path) -> Result<Draft, Problem> {
    let file = File::open(path).map_err(|_| Problem::Unreadable)?;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE as u64 + BOM.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Problem::Unreadable)?;
    decode(&bytes).map(Draft::new)
}

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// The text the grid will receive: no byte order mark (it would reach the
/// compiler as a stray character), `\n` line ends (the Legacy editor reads
/// the file in text mode), valid UTF-8, not blank, within the grid's limit.
/// The limit applies to the file's bytes after the mark, which the line-end
/// conversion can only shrink.
pub fn decode(bytes: &[u8]) -> Result<String, Problem> {
    let bytes = bytes.strip_prefix(&BOM).unwrap_or(bytes);
    if bytes.len() > MAX_SOURCE {
        return Err(Problem::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Problem::NotUtf8)?;
    if text.trim().is_empty() {
        return Err(Problem::Empty);
    }
    Ok(normalize_newlines(text))
}

/// CRLF and a lone CR become LF.
pub fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Lines of a text: a last line without a line end counts, a trailing line
/// end does not start another.
pub fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// The first `lines` lines of `text` (without a cut line end), and whether
/// anything follows.
pub fn head(text: &str, lines: usize) -> (&str, bool) {
    let mut end = 0;
    for (count, line) in text.split_inclusive('\n').enumerate() {
        if count == lines {
            return (text[..end].trim_end_matches('\n'), true);
        }
        end += line.len();
    }
    (text.trim_end_matches('\n'), false)
}

/// The inventory name of a script file: its stem, cleaned like every upload
/// label (`generated_name`, LLFloaterNameDesc::postBuild) to the 63 characters
/// of an inventory name, or a default when nothing usable is left.
pub fn item_name(path: &Path) -> String {
    let name = crate::text_validation::generated_name(&path.file_stem().unwrap_or_default().to_string_lossy(), 63);
    if name.is_empty() { DEFAULT_NAME.to_owned() } else { name }
}

/// What happened to one script of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub name: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Saved,
    /// Saved, but the grid could not compile it: the item stays, as in the
    /// Legacy editor.
    CompileErrors(Vec<String>),
    Failed(String),
}

/// What the importer keeps of the answers for the script being created: the
/// request it waits for, so that an answer for any other (a recovered script,
/// an earlier cancelled one) is never taken for it.
#[derive(Default)]
pub(super) struct Pending {
    errors: Option<(Uuid, Vec<String>)>,
}

impl Pending {
    pub(super) fn errors(&mut self, request: Uuid, errors: Vec<String>) {
        self.errors = Some((request, errors));
    }

    /// The compile errors reported for `request`; any other is dropped.
    pub(super) fn take(&mut self, request: Uuid) -> Vec<String> {
        match self.errors.take() {
            Some((id, errors)) if id == request => errors,
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod tests;
