//! The script editor: the documents a window holds and what each keeps (text,
//! cursor, folds, undo, Find, its colour picker), the window that shows them --
//! a linkset's tree, its tabs and its split, the docks under the source, the
//! status bar and the documentation pane -- and what a save, a load or a
//! compile target answers. Each document owns its pending requests and saved
//! snapshot, so late replies cannot replace another editor's text. The Legacy
//! Viewer opens one floater per script (LLPreviewLSL, LLLiveLSLEditor,
//! indra/newview); Aurora shares one window unless Preferences › Script editor
//! asks for one per script.
//!
//! The scripts of one linkset share a window: the tree of the linkset beside
//! the source, a star on the row of a script with unsaved text, and the menus
//! and toolbar above both (`linkset.rs`), Aurora's own layout where the Legacy
//! Viewer opens one floater per script.

use super::widgets::{Floater, flat_button};
use crate::theme::Palette;
use aurora_net::scripts::{CompileTarget, MAX_SOURCE, ScriptOptions, ScriptSaved, ScriptTarget};
use egui::Vec2;
use uuid::Uuid;

/// The lines a window's Console keeps: a script that says a line a frame
/// cannot grow it without end.
/// Two documents of the list, mutably, in the order given: the two panes of a
/// split window.
fn two_mut(documents: &mut [Editor], a: usize, b: usize) -> (&mut Editor, &mut Editor) {
    debug_assert_ne!(a, b);
    if a < b {
        let (left, right) = documents.split_at_mut(b);
        (&mut left[a], &mut right[0])
    } else {
        let (left, right) = documents.split_at_mut(a);
        (&mut right[0], &mut left[b])
    }
}

pub mod backup;
mod colors;
mod complete;
mod create;
mod crumbs;
mod debug;
mod demo;
mod dialogs;
pub(crate) mod editing;
mod experiences;
mod files;
pub(crate) use files::live_folder;
mod focus;
mod folding;
mod format;
mod indent;
mod inlays;
mod linkset;
mod minimap;
mod navigate;
mod outline;
mod preprocessor;
pub mod recovery;
mod reference;
pub mod settings;
mod syntax;
mod view;

pub use focus::editor_focused;
pub(crate) use indent::{TAB_SIZE, edit_scoped as edit_code};
pub use linkset::{
    Linkset, Prim as LinksetPrim, PrimKind, PrimMenu as LinksetPrimMenu, Script as LinksetScript, ScriptMenu as LinksetScriptMenu,
};
pub(crate) use linkset::{root_row_id, script_row_id};
pub(crate) use syntax::highlight as highlight_lsl;

pub use view::BarChange;

/// The key of the window of the agent's own scripts: one window for the
/// inventory's scripts, grouped by the folder that holds each, drawn with the
/// same tree, tabs and dock as an object's linkset. A fixed id that no object
/// or item of a grid has (theirs are random), and not nil, the key of a draft's
/// own window on the login screen.
pub const INVENTORY_WINDOW: Uuid = Uuid::from_u128(0x5C01_9C0D_0000_0000_0000_0000_0000_0001);

pub enum Action {
    /// Rename only the inventory/task item metadata, never the source text.
    Rename {
        target: ScriptTarget,
        name: String,
    },
    Include {
        request: Uuid,
        name: String,
    },
    Paste(ScriptTarget),
    SettingsChanged,
    Save {
        request: Uuid,
        target: ScriptTarget,
        options: ScriptOptions,
        source: String,
    },
    Reload {
        request: Uuid,
        target: ScriptTarget,
    },
    Experiences {
        request: Uuid,
        target: ScriptTarget,
    },
    Running {
        target: ScriptTarget,
        running: bool,
    },
    Reset(ScriptTarget),
    QueryRunning(ScriptTarget),
    /// A script of the linkset's tree that is not open yet: the app gives it
    /// an editor.
    OpenScript(ScriptTarget),
    /// "Copy to inventory" of a script's contents menu, the same action as in
    /// the contents window.
    TakeScript {
        object: Uuid,
        item: Uuid,
    },
    /// "Delete" of a script's contents menu.
    DeleteScript {
        object: Uuid,
        item: Uuid,
    },
    /// "Properties" of a script's contents menu: the item's own window.
    Properties(ScriptTarget),
    /// "New script" of a prim's menu in the tree, and the strip's plus: the
    /// language chosen on the new script's page, which the app creates in
    /// `object` (`lua` for an SLua script).
    NewScript {
        object: Uuid,
        lua: bool,
    },
    /// Take / Take copy of the linkset's root, from its menu in the tree: the
    /// object menu's own action for that object.
    TakeObject {
        object: Uuid,
        copy: bool,
    },
    /// Delete of the linkset's root, from its menu in the tree.
    DeleteObject(Uuid),
    /// A touch on a prim, from its menu in the tree or a click in the tree's
    /// 3D view: the object menu's own touch, refused by RLVa where it is.
    Touch(Uuid),
    /// "Edit" of a prim's menu in the tree: the build floater's Edit tool, on
    /// that prim.
    EditObject(Uuid),
    /// Save every script the window has open: the linkset's (`linkset` not
    /// nil), or the one document of a window of its own.
    SaveAll {
        linkset: Uuid,
        target: ScriptTarget,
    },
    /// The status bar's own controls: the editor's zoom, the wrapping, the
    /// minimap (the app keeps them in the script settings).
    BarChange(view::BarChange),
    /// A close prompt was cancelled (or its window closed): a quit that waited
    /// for the scripts' text gives up.
    CloseCancelled,
}

/// The script a page's prim gained since the page opened: the first one that
/// was not there (`before`), which is the one the page asked for. The app's
/// listing is the only word on what the object made.
fn new_script_of<'a>(scripts: &'a [linkset::Script], before: &[Uuid]) -> Option<&'a linkset::Script> {
    scripts.iter().find(|script| !before.contains(&script.item))
}

/// A script's text read for an object-wide search or check, no document
/// opened: its request while it loads, then its text or what stopped it.
#[derive(Default)]
struct ReadText {
    request: Option<Uuid>,
    text: Option<String>,
    error: Option<String>,
}

/// One line of a window's Console tab: when the object said it with
/// `llOwnerSay`, the name it spoke with (a window of a linkset holds several
/// prims), and what it said.
/// What the region reports of an object's scripts, as the bridge brings it
/// (`OBJECT_SCRIPT_MEMORY`, `OBJECT_RUNNING_SCRIPT_COUNT`,
/// `OBJECT_TOTAL_SCRIPT_COUNT`): the memory they are allowed, not their live
/// heaps, for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptStats {
    /// The memory the simulator sets aside for the object's scripts, in bytes.
    pub memory: u32,
    pub running: u32,
    pub total: u32,
}

struct PendingSave {
    request: Uuid,
    source: String,
    compile: CompileTarget,
    experience: Option<Uuid>,
    running: bool,
}

pub struct Editor {
    pub target: ScriptTarget,
    /// The linkset this script belongs to, when it is inside an object: the
    /// scripts of one linkset share a window and its tree. Nil for an agent
    /// inventory script.
    pub linkset: Uuid,
    /// A script that does not exist yet: the page that asks which language
    /// (`create::Draft`). Such a document has no item, no text and nothing to
    /// save; it leaves once a language is chosen.
    draft: Option<create::Draft>,
    /// Inventory list that opened this editor, if any.
    pub(crate) inventory_owner: Option<egui::Id>,
    pub name: String,
    pub source: String,
    highlighter: syntax::Highlighter,
    colors: colors::State,
    folding: folding::State,
    baseline: String,
    loaded: bool,
    loading: Option<Uuid>,
    saving: Option<PendingSave>,
    pub can_read: bool,
    pub can_edit: bool,
    pub can_reset: bool,
    pub compile: CompileTarget,
    baseline_compile: CompileTarget,
    experience: experiences::State,
    lua_scripts: bool,
    running: Option<bool>,
    status: String,
    /// When this script was last saved in this session, for the bar to say so.
    pub(super) saved_at: Option<std::time::SystemTime>,
    errors: Vec<String>,
    compile_failed: bool,
    close_prompt: bool,
    close_after_save: bool,
    /// The lines a reader marked in the gutter (0-based, as the text is), for
    /// F2 and Shift+F2.
    bookmarks: std::collections::BTreeSet<usize>,
    closed: bool,
    raise: bool,
    tools: view::Tools,
    recompile_allowed: bool,
    processor_settings: settings::ScriptSettings,
    processor_environment: preprocessor::Environment,
    processing: Option<preprocessor::Job>,
}

impl Editor {
    /// Whether the window still shows this script.
    pub(crate) fn is_open(&self) -> bool {
        !self.closed
    }

    /// Its text has changes the item does not have yet.
    pub(crate) fn unsaved(&self) -> bool {
        self.dirty()
    }

    /// SLua source: Lua colors, words and help.
    /// The language this script is written in.
    fn lang(&self) -> Lang {
        Lang::of(self.compile)
    }

    fn lua(&self) -> bool {
        self.lang().is_lua()
    }

    /// The number of the first line: Lua counts from 1, LSL from 0
    /// (LLScriptEditor::drawLineNumbers, LLScriptEdCore::updateDynamicHelp).
    fn first_line(&self) -> usize {
        usize::from(self.lua())
    }

    fn dirty(&self) -> bool {
        self.loaded && (self.source != self.baseline || self.compile != self.baseline_compile || self.experience.dirty())
    }

    /// The hash of the text the window shows -- the preprocessed one while the
    /// Output view is on -- made once a frame: `stamp` is that frame's clock, so
    /// that the analysis, the minimap, the inlays and the weigh all read one
    /// hash instead of walking the script each time they ask. `None` (a save,
    /// outside a frame) makes it again.
    fn text_key(&mut self, stamp: Option<f64>) -> u64 {
        use std::hash::{Hash, Hasher};
        if let (Some(stamp), Some((made, key))) = (stamp, self.tools.text_key)
            && made == stamp
        {
            return key;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.source.hash(&mut hasher);
        if self.tools.processed {
            self.tools.processed_source.hash(&mut hasher);
        }
        let key = hasher.finish();
        if let Some(stamp) = stamp {
            self.tools.text_key = Some((stamp, key));
        }
        key
    }

    /// The script's outline, the linter's rows and the blocks of its text, made
    /// again only when the text, its name or its language changes: the dock, the
    /// sticky line, the gutter and the path under the caret read them every
    /// frame, and the linter is not for every frame. See `text_key` for `stamp`.
    fn analysis(&mut self, stamp: Option<f64>) -> &view::Analysis {
        use std::hash::{Hash, Hasher};
        let lua = self.lua();
        let lang = self.lang();
        let key = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.text_key(stamp).hash(&mut hasher);
            self.name.hash(&mut hasher);
            lua.hash(&mut hasher);
            hasher.finish()
        };
        if self.tools.analysis.0 != Some(key) {
            let made = if self.tools.processed {
                view::Analysis::make(&self.tools.processed_source, lang)
            } else {
                view::Analysis::make(&self.source, lang)
            };
            self.tools.analysis = (Some(key), made);
        }
        // just made, or kept from an earlier frame of the same text
        &self.tools.analysis.1
    }

    /// Closes the document -- its colour picker's pending click goes with it --
    /// or asks about the text first when it has changes the item has not, or a
    /// save on its way: LLFloaterScriptEd's close check.
    fn close_asking(&mut self) {
        self.colors.cancel(&mut self.source);
        if self.dirty() || self.saving.is_some() {
            self.close_prompt = true;
        } else {
            self.closed = true;
        }
    }

    fn can_save(&self) -> bool {
        self.loaded
            && self.can_edit
            && !self.colors.active()
            && self.saving.is_none()
            && self.loading.is_none()
            && (self.target.object.is_nil()
                || (self.running.is_some() && self.experience.selected.is_some() && self.experience.loading.is_none()))
            && (self.dirty() || self.compile_failed || self.recompile_allowed)
            && self.source.len() <= MAX_SOURCE
    }

    fn save(&mut self) -> Option<Action> {
        if !self.can_save() {
            return None;
        }
        let request = Uuid::new_v4();
        self.saving = Some(PendingSave {
            request,
            source: self.source.clone(),
            compile: self.compile,
            experience: self.experience.selected,
            running: self.running.unwrap_or(false),
        });
        self.status = aurora_i18n::tr!("script-saving").into();
        self.errors.clear();
        self.tools.diagnostics_processed = false;
        // the LSL preprocessor would mangle SLua source
        if self.processor_settings.preprocessor && self.compile != CompileTarget::Luau {
            self.status = aurora_i18n::tr!("script-preproc-processing").into();
            self.processing = Some(preprocessor::start(
                self.source.clone(),
                self.name.clone(),
                self.processor_settings.clone(),
                self.processor_environment.clone(),
            ));
            return None;
        }
        Some(Action::Save {
            request,
            target: self.target,
            options: ScriptOptions {
                compile: self.compile,
                running: self.running.unwrap_or(false),
                experience: self.experience.selected.unwrap_or_default(),
            },
            source: self.source.clone(),
        })
    }
}

/// The language a script is written in: what its scanners read it as, and what
/// its catalogue names (`llSay` or `ll.Say`). The Luau VM runs Lua.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Lang {
    #[default]
    Lsl,
    Lua,
}

impl Lang {
    /// The language a compile target's text is written in.
    pub(crate) fn of(compile: aurora_net::scripts::CompileTarget) -> Self {
        if compile == aurora_net::scripts::CompileTarget::Luau {
            Lang::Lua
        } else {
            Lang::Lsl
        }
    }

    pub(crate) fn is_lua(self) -> bool {
        self == Lang::Lua
    }
}

/// The 3D view of the object a script window's tree is about, beside its
/// Properties: the prims the app feeds the stage (`object_stage`), and the one
/// the script in front is in, outlined.
#[derive(Default)]
pub struct StageView {
    pub view: crate::ui::object_stage::StageUi,
    /// The object's root prim, and the prim the script in front is in.
    pub root: Uuid,
    pub prim: Uuid,
    /// The names of the prims shown, for the tag under the pointer.
    pub names: Vec<(Uuid, String)>,
}

#[derive(Default)]
pub struct Editors {
    pub documents: Vec<Editor>,
    /// The layout the open scripts were put in, as the preferences last said:
    /// one window per script, or the shared windows. The app watches it, so a
    /// change moves every open script to the window it belongs in now.
    pub simple: bool,
    /// The script shown in each linkset window, by the linkset's root: the
    /// one the user chose in the tree, or the last one opened.
    active: std::collections::HashMap<Uuid, ScriptTarget>,
    /// The filter of each linkset window's tree, by its root.
    filters: std::collections::HashMap<Uuid, String>,
    /// The scripts read for an object-wide search or check, no document
    /// opened, by the script; and the request being served, by its id.
    read: std::collections::HashMap<ScriptTarget, ReadText>,
    reads: std::collections::HashMap<Uuid, ScriptTarget>,
    /// The two scripts a window shows side by side, left then right, by its
    /// root. The one in front is one of them.
    splits: std::collections::HashMap<Uuid, (ScriptTarget, ScriptTarget)>,
    /// How much of the split's width its left pane takes, by the window's
    /// root (a half until the divider is dragged).
    split_ratio: std::collections::HashMap<Uuid, f32>,
    /// The split a window waits for: a script dragged from the tree onto a
    /// pane before its document existed, and the pane it took beside the other
    /// one, by the window's root, until the app opens it.
    pending_split: std::collections::HashMap<Uuid, (ScriptTarget, ScriptTarget)>,
    /// The scripts a window's tree has selected, by its root, in the order
    /// they were chosen; the script in front is one of them (a plain click
    /// leaves it alone).
    tree_selection: std::collections::HashMap<Uuid, Vec<ScriptTarget>>,
    /// The region's figure of each object's scripts, by the object (the
    /// window's root), as the bridge last brought it.
    stats: std::collections::HashMap<Uuid, ScriptStats>,
    /// The scripts of each window's tab strip, in the order the reader dragged
    /// them into, by the window's root.
    tab_order: std::collections::HashMap<Uuid, Vec<ScriptTarget>>,
    /// The prim each window's tree is to open because the script in it was
    /// just chosen: a script brought to the front shows in the tree.
    reveal: std::collections::HashMap<Uuid, Uuid>,
    /// The 3D view of the object each window's tree is about, by the window's
    /// root: the app keeps its stage up to date from the world.
    pub stages: std::collections::HashMap<Uuid, StageView>,
    panels: dialogs::Panels,
    debug: debug::Debug,
    includes: std::collections::HashMap<Uuid, PendingInclude>,
    pub agent: Uuid,
    pub agent_name: String,
    /// The region runs SLua: the Lua compile targets can be chosen.
    pub lua_scripts: bool,
    /// The account's folder of copies of the unsaved scripts (backup.rs);
    /// `None` keeps none.
    pub backup_dir: Option<std::path::PathBuf>,
    pub backups: backup::Book,
    /// The saves of scripts sent to the external editor, for the editor
    /// sync (`crate::editor_sync`), taken once a frame.
    compiled: Vec<Compiled>,
}

/// A script sent to the external editor (or on its way there), which the
/// editor sync may let an editor follow.
pub struct Synced<'a> {
    pub target: ScriptTarget,
    pub name: &'a str,
    pub lua: bool,
    pub can_edit: bool,
}

/// A save's result (LLScriptEdContainer::sendCompileResults).
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub target: ScriptTarget,
    pub compiled: bool,
    /// An object's script runs; false in the inventory.
    pub running: bool,
    pub errors: Vec<String>,
    pub lua: bool,
}

/// Saves kept for the editor sync between two frames.
const MAX_COMPILED: usize = 32;

struct PendingInclude {
    owner: ScriptTarget,
    asset: Uuid,
    reply: std::sync::mpsc::Sender<Result<preprocessor::Include, String>>,
}

/// The tab strip's order once `moved` is dropped on `onto`: before it when it
/// comes from its right, after it when it comes from its left.
fn reorder_targets(order: &mut Vec<ScriptTarget>, moved: ScriptTarget, onto: ScriptTarget) {
    let Some(from) = order.iter().position(|target| *target == moved) else {
        return;
    };
    let Some(at) = order.iter().position(|target| *target == onto) else {
        return;
    };
    if from == at {
        return;
    }
    order.remove(from);
    let anchor = order
        .iter()
        .position(|target| *target == onto)
        .unwrap_or(order.len().saturating_sub(1));
    let slot = if from < at { anchor + 1 } else { anchor };
    order.insert(slot.min(order.len()), moved);
}

impl Editors {
    pub fn show_debug(&mut self, ctx: &egui::Context, p: &Palette, messages: &mut crate::world::script_debug::Messages) {
        self.debug.show(ctx, p, messages);
    }
    pub fn open_debug(&mut self) {
        self.debug.open = true;
    }

    /// Copy the unsaved scripts (backup.rs), at most every
    /// `backup::INTERVAL`, off the frame thread.
    pub fn back_up(&mut self) {
        self.back_up_now(false);
    }

    /// The viewer closing: as [`Self::back_up`], and a copy whose last
    /// write was refused is tried again now rather than at the interval.
    pub fn back_up_before_exit(&mut self) {
        self.back_up_now(backup::any_refused());
    }

    fn back_up_now(&mut self, force: bool) {
        let docs = self.documents.iter().map(|e| backup::Doc {
            target: e.target,
            name: &e.name,
            source: &e.source,
            unsaved: e.loaded && e.source != e.baseline,
            editable: e.loaded && e.can_edit,
        });
        let jobs = self
            .backups
            .plan(self.backup_dir.as_deref(), docs, std::time::Instant::now(), force);
        backup::run(jobs);
    }

    /// The scripts sent to the external editor.
    pub fn synced(&self) -> impl Iterator<Item = Synced<'_>> {
        self.documents
            .iter()
            .filter(|e| !e.closed && (e.tools.external.is_some() || e.tools.external_starting))
            .map(|e| Synced {
                target: e.target,
                name: &e.name,
                lua: e.lua(),
                can_edit: e.can_edit,
            })
    }

    /// The saves since the last call.
    pub fn take_compiled(&mut self) -> Vec<Compiled> {
        std::mem::take(&mut self.compiled)
    }

    /// `AURORA_DEMO_SCRIPTS=split`: the window's two open scripts side by
    /// side, the divider a little left of the middle.
    /// `AURORA_DEMO_SCRIPTS=outline`: the window's dock on its Outline tab,
    /// the tree of the script in front.
    /// `AURORA_DEMO_SCRIPTS=new`: the page that asks which language a new
    /// script of `root` will be written in, for `prim` (`lua` where the demo
    /// region runs SLua).
    /// The objects whose windows are open and whose script figure the status
    /// bar shows: the app asks the bridge for them (`editor_script_info`).
    pub fn stats_wanted(&self) -> Vec<Uuid> {
        let mut wanted = Vec::new();
        for e in self.documents.iter().filter(|e| !e.closed && !e.linkset.is_nil()) {
            let root = Self::window_key(e);
            if !wanted.contains(&root) {
                wanted.push(root);
            }
        }
        wanted.truncate(4);
        wanted
    }

    /// Keeps the region's figure of an object's scripts, for its window's bar.
    pub fn script_stats(&mut self, info: &aurora_net::bridge::query::ScriptInfo) {
        self.stats.insert(
            info.object,
            ScriptStats {
                memory: info.memory,
                running: info.running,
                total: info.total,
            },
        );
    }

    /// The figure of `root`'s object's scripts, when the region has given one.
    fn stats_of(&self, root: Uuid) -> Option<ScriptStats> {
        self.stats.get(&root).copied()
    }

    /// `AURORA_DEMO_SCRIPTS=lints`: the window's dock on its Problems tab,
    /// unfolded, so that the findings show at once.
    pub fn disconnected(&mut self) {
        // the latest text is copied, and the copies stay for the next login
        // whatever the editors do now (LLScriptEdContainer's destructor
        // keeps the file when there is no region)
        self.back_up_now(true);
        self.backups.release();
        self.includes.clear();
        self.documents.retain(|e| e.dirty() || e.saving.is_some());
        for e in &mut self.documents {
            // the linkset is gone with the connection: each kept draft gets
            // its own window again, with its name
            e.linkset = Uuid::nil();
            e.colors.cancel(&mut e.source);
            e.loading = None;
            e.experience.loading = None;
            e.saving = None;
            e.processing = None;
            e.tools.external = None;
            e.tools.external_starting = false;
            e.can_read = false;
            e.can_edit = false;
            e.can_reset = false;
            e.running = None;
            e.close_prompt = false;
            e.close_after_save = false;
            e.status = aurora_i18n::tr!("script-disconnected").into();
        }
    }

    pub fn prepare_exit(&mut self) -> bool {
        let mut ready = true;
        for e in &mut self.documents {
            // a script already closed (discarded, or its window's X) owes
            // nothing more: asking again would hold the quit for ever
            if e.closed {
                continue;
            }
            e.colors.cancel(&mut e.source);
            if e.dirty() || e.saving.is_some() {
                e.close_prompt = true;
                ready = false;
            }
        }
        ready
    }

    /// How many open scripts ask about their text (the `quit` demo).
    pub fn prompting(&self) -> usize {
        self.documents.iter().filter(|e| !e.closed && e.close_prompt).count()
    }

    /// What the Discard button of a close prompt does, for every script asking
    /// (the `quit` demo drives the whole flow without clicks; the buttons
    /// themselves are the test's).
    /// Every open script is settled: nothing more to save, to ask about, or to
    /// wait for. A quit held by `prepare_exit` goes on once this is true.
    pub fn exit_ready(&self) -> bool {
        self.documents
            .iter()
            .all(|e| e.closed || (!e.close_prompt && !e.dirty() && e.saving.is_none()))
    }

    /// AURORA_DEMO_EDITOR_SYNC: the script counts as sent to the external
    /// editor without launching one (the demo's editor is a socket client).
    /// The demo of the Code menu: a click on its button, as a reader's.
    /// `AURORA_DEMO_SCRIPTS=docs` and its siblings: what the documentation
    /// pane shows, in the window's own state (the pane opened on a function,
    /// on a constant the catalogue says nothing about, or closed).
    /// `AURORA_DEMO_SCRIPTS=search-linkset`: Find on one script of a
    /// linkset's window, with a query the demo's scripts answer.
    fn focus_source(e: &Editor, ctx: &egui::Context, byte: usize) {
        let cursor = egui::text::CCursor::new(e.source[..byte].chars().count());
        let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
        let text_id = egui::Id::unique((&id, "source_text"));
        if let Some(mut state) = egui::TextEdit::load_state(ctx, text_id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(cursor)));
            state.store(ctx, text_id);
            ctx.memory_mut(|m| m.request_focus(text_id));
        }
    }

    /// Opens the editor of a script item, or raises it. `flags` are the
    /// item's: its language sets the target of a new editor only, so a
    /// reopened one keeps what was chosen and saved (`CompileTarget::for_item`).
    pub fn open(&mut self, target: ScriptTarget, name: String, can_read: bool, can_edit: bool, flags: u32) -> Option<Uuid> {
        for editor in &mut self.documents {
            editor.tools.focus_start = false;
        }
        if let Some(editor) = self.documents.iter_mut().find(|e| e.target == target) {
            editor.raise = true;
            editor.tools.focus_start = true;
            return None;
        }
        let request = can_read.then(Uuid::new_v4);
        self.documents.push(Editor {
            target,
            linkset: Uuid::nil(),
            draft: None,
            inventory_owner: None,
            name,
            source: String::new(),
            highlighter: syntax::Highlighter::default(),
            colors: colors::State::new(format!("script_color_picker_{}_{}", target.object, target.item)),
            folding: folding::State::default(),
            baseline: String::new(),
            loaded: false,
            loading: request,
            saving: None,
            can_read,
            can_edit,
            can_reset: !target.object.is_nil(),
            compile: CompileTarget::for_item(flags),
            baseline_compile: CompileTarget::for_item(flags),
            experience: experiences::State::default(),
            lua_scripts: false,
            running: None,
            status: if can_read {
                aurora_i18n::tr!("common-loading-2")
            } else {
                aurora_i18n::tr!("script-no-permission")
            }
            .into(),
            saved_at: None,
            errors: Vec::new(),
            compile_failed: false,
            close_prompt: false,
            close_after_save: false,
            bookmarks: std::collections::BTreeSet::new(),
            closed: false,
            raise: true,
            tools: view::Tools::opening(),
            recompile_allowed: false,
            processor_settings: settings::ScriptSettings::default(),
            processor_environment: preprocessor::Environment::default(),
            processing: None,
        });
        request
    }

    /// Opens the page that asks which language a new script of `object` will
    /// be written in, in place of the one this window had: the tree's "New
    /// script" and the strip's plus both come here, while the item itself is
    /// created only once a language is chosen
    /// (LLPanelContents::onNewScriptFlyout, indra/newview/llpanelcontents.cpp).
    /// `before` are the scripts `object` held when the page opened: the one
    /// that appears past them is the new script, and opens (`Action::NewScript`).
    fn open_draft(&mut self, root: Uuid, object: Uuid, prim: String, lang: Lang, before: Vec<Uuid>) {
        let lua = lang.is_lua();
        // one page per window: asking from another prim moves it there
        self.documents.retain(|e| e.draft.is_none() || e.linkset != root);
        let draft = create::Draft {
            object,
            prim,
            lua,
            before,
            chosen: false,
            chosen_at: 0.0,
        };
        let target = draft.target();
        let name = aurora_i18n::tr!("extra-new-script").to_string();
        self.documents.push(Editor {
            target,
            linkset: root,
            draft: Some(draft),
            inventory_owner: None,
            name,
            source: String::new(),
            highlighter: syntax::Highlighter::default(),
            colors: colors::State::new(format!("script_color_picker_{}_{}", target.object, target.item)),
            folding: folding::State::default(),
            baseline: String::new(),
            // nothing to load: it is a page, not an item of the object
            loaded: true,
            loading: None,
            saving: None,
            can_read: false,
            can_edit: false,
            can_reset: false,
            compile: CompileTarget::Mono,
            baseline_compile: CompileTarget::Mono,
            experience: experiences::State::default(),
            lua_scripts: false,
            running: None,
            status: String::new(),
            saved_at: None,
            errors: Vec::new(),
            compile_failed: false,
            close_prompt: false,
            close_after_save: false,
            bookmarks: std::collections::BTreeSet::new(),
            closed: false,
            raise: true,
            tools: view::Tools::opening(),
            recompile_allowed: false,
            processor_settings: settings::ScriptSettings::default(),
            processor_environment: preprocessor::Environment::default(),
            processing: None,
        });
        self.active.insert(root, target);
    }

    pub fn loaded(&mut self, request: Uuid, target: ScriptTarget, result: Result<String, String>) {
        // a script read for a search or a check has no document
        if let Some(target) = self.reads.remove(&request) {
            if let Some(read) = self.read.get_mut(&target) {
                read.request = None;
                match result {
                    Ok(text) => read.text = Some(text),
                    Err(reason) => read.error = Some(reason),
                }
            }
            return;
        }
        if let Some(include) = self.includes.remove(&request) {
            let _ = include.reply.send(result.map(|source| preprocessor::Include {
                source,
                asset: include.asset,
            }));
            return;
        }
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target && e.loading == Some(request)) else {
            return;
        };
        e.loading = None;
        match result {
            Ok(source) => {
                e.tools.reset_history = true;
                e.tools.processed_source.clone_from(&source);
                let source = preprocessor::decode(&source);
                e.tools.has_processed = source != e.tools.processed_source;
                e.tools.diagnostics_processed = false;
                e.baseline.clone_from(&source);
                e.source = source;
                e.loaded = true;
                e.compile_failed = false;
                e.errors.clear();
                e.status.clear();
            }
            Err(reason) => e.status = reason,
        }
    }

    pub fn load_experiences(&mut self, target: ScriptTarget) -> Option<Action> {
        let e = self
            .documents
            .iter_mut()
            .find(|e| e.target == target && e.can_read && !target.object.is_nil())?;
        let request = e.experience.begin()?;
        Some(Action::Experiences { request, target })
    }

    pub fn include_asset(&mut self, request: Uuid, asset: Uuid) {
        if let Some(include) = self.includes.get_mut(&request) {
            include.asset = asset;
        }
    }

    pub fn experiences_loaded(&mut self, request: Uuid, target: ScriptTarget, result: aurora_net::scripts::experiences::ScriptExperiences) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
            e.experience.loaded(request, result);
        }
    }

    pub fn saved(&mut self, request: Uuid, target: ScriptTarget, result: &Result<ScriptSaved, String>) -> bool {
        let Some(e) = self
            .documents
            .iter_mut()
            .find(|e| e.target == target && e.saving.as_ref().is_some_and(|s| s.request == request))
        else {
            return false;
        };
        let Some(pending) = e.saving.take() else { return false };
        // finishedLSLUpload / failedLSLUpload: the result goes to the
        // editor that follows the script
        if (e.tools.external.is_some() || e.tools.external_starting) && self.compiled.len() < MAX_COMPILED {
            let lua = pending.compile == CompileTarget::Luau;
            self.compiled.push(match result {
                Ok(saved) => Compiled {
                    target,
                    compiled: saved.compiled,
                    running: !target.object.is_nil() && pending.running,
                    errors: saved.errors.clone(),
                    lua,
                },
                Err(reason) => Compiled {
                    target,
                    compiled: false,
                    running: false,
                    errors: vec![aurora_i18n::tr!("script-sync-upload-failed", reason = reason.as_str())],
                    lua,
                },
            });
        }
        let outcome = match result {
            Ok(saved) => {
                e.baseline = pending.source;
                e.baseline_compile = pending.compile;
                e.experience.baseline = pending.experience;
                e.compile_failed = !saved.compiled;
                e.errors.clone_from(&saved.errors);
                e.saved_at = Some(std::time::SystemTime::now());
                if e.close_after_save && saved.compiled && !e.dirty() {
                    e.closed = true;
                }
                if !saved.compiled {
                    aurora_i18n::tr!("script-compile-failed").to_string()
                } else if saved.checked {
                    aurora_i18n::tr!("script-saved").to_string()
                } else {
                    aurora_i18n::tr!("script-saved-not-compiled").to_string()
                }
            }
            Err(reason) => reason.to_string(),
        };
        e.status = outcome;
        e.close_after_save = false;
        true
    }

    pub fn running(&mut self, target: ScriptTarget, running: bool, compile: Option<CompileTarget>) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
            // LLLiveLSLEditor::processScriptRunningReply: the first reply
            // names the VM. A Lua source (its item subtype, which set Luau
            // when the editor opened) stays Lua whatever the reply says:
            // saved as LSL it would fail or break.
            if e.running.is_none()
                && e.baseline_compile != CompileTarget::Luau
                && let Some(compile) = compile
            {
                e.compile = compile;
                e.baseline_compile = compile;
            }
            e.running = Some(running);
        }
    }

    pub fn paste(&mut self, ctx: &egui::Context, target: ScriptTarget, text: &str) {
        let Some(e) = self
            .documents
            .iter_mut()
            .find(|e| e.target == target && e.can_edit && e.loaded && e.saving.is_none() && !e.tools.processed)
        else {
            return;
        };
        let id = egui::Id::unique((&format!("script_editor_{}_{}", target.object, target.item), "source_text"));
        let [a, b] = editing::cursor(ctx, id).sorted_cursors();
        if !editing::replace(ctx, id, &mut e.source, a.index.0..b.index.0, text) {
            e.status = aurora_i18n::tr!("script-source-too-large").into();
        }
    }

    pub fn color_picker(&mut self) -> Option<&mut super::color_picker::ColorPicker> {
        let index = self
            .documents
            .iter()
            .position(|e| e.colors.picker.pipette)
            .or_else(|| self.documents.iter().position(|e| e.colors.picker.is_open()))?;
        Some(&mut self.documents[index].colors.picker)
    }

    pub fn color_pipette(&self) -> bool {
        self.documents.iter().any(|e| e.colors.picker.pipette)
    }

    pub fn color_open(&self) -> bool {
        self.documents.iter().any(|e| e.colors.picker.is_open())
    }

    /// The identity of a document's own parts: its source, its dialogs and
    /// the commands its controls leave for the next pass.
    pub(super) fn document_id(e: &Editor) -> String {
        format!("script_editor_{}_{}", e.target.object, e.target.item)
    }

    /// The floater a document is drawn in: its own (a draft of the login
    /// screen), or the one its window's scripts share -- an object's linkset,
    /// or the agent's inventory.
    pub(super) fn window_of(e: &Editor) -> String {
        if e.linkset.is_nil() {
            Self::document_id(e)
        } else {
            Self::window_name(e.linkset)
        }
    }

    /// The prim each script in a window of its own says it is in, for the
    /// first chip under its source: a window of the simple layout has no tree
    /// to ask, and a script of the inventory has no prim.
    pub fn name_own_scripts(&mut self, prim: impl Fn(ScriptTarget) -> String) {
        for e in &mut self.documents {
            if e.is_open() && e.linkset.is_nil() && !e.target.object.is_nil() {
                e.tools.prim = prim(e.target);
            }
        }
    }

    /// The floater's name for one window's key: the inventory's own, or the
    /// linkset's.
    pub(crate) fn window_name(root: Uuid) -> String {
        if root == INVENTORY_WINDOW {
            "script_inventory".into()
        } else {
            format!("script_linkset_{root}")
        }
    }

    /// The key of a window in the maps of per-window state: its linkset's
    /// root, or the script's item when it has a window of its own.
    fn window_key(e: &Editor) -> Uuid {
        if e.linkset.is_nil() { e.target.item } else { e.linkset }
    }

    /// The script in front in a window: the app feeds its tree's 3D view the
    /// prim that one is in.
    pub fn script_in_front(&self, window: Uuid) -> Option<ScriptTarget> {
        self.active.get(&window).copied()
    }

    /// The window commands of the script in front, while its source has the
    /// keyboard (Ctrl+F is `focus::find`'s): Ctrl+Tab and Ctrl+Shift+Tab cycle
    /// the scripts of its window, Ctrl+W closes it, Ctrl+1 to Ctrl+5 choose a
    /// tab of the dock in the reader's order. Run before the sources are
    /// drawn, or the text edit would take the Tab as an indentation.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let Some(owner) = focus::active(ctx) else { return };
        // only from the source: a field of the tree (its filter) keeps the
        // text keys, and Find has its own
        if !ctx.memory(|m| m.has_focus(owner)) {
            return;
        }
        let Some(index) = self
            .documents
            .iter()
            .position(|e| !e.closed && egui::Id::unique((&Self::document_id(e), "source_text")) == owner)
        else {
            return;
        };
        #[derive(Clone, Copy)]
        enum Step {
            Next,
            Previous,
            Close,
            Split,
        }
        let step = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Tab) {
                Some(Step::Previous)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Tab) {
                Some(Step::Next)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Backslash) {
                Some(Step::Split)
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::W) {
                // AZERTY types its Z on the physical W: egui's logical keys
                // leave that press to the field as Ctrl+Z, so undo keeps it
                // (text_field_keeps)
                Some(Step::Close)
            } else {
                None
            }
        });
        match step {
            Some(Step::Next | Step::Previous) => {
                let linkset = self.documents[index].linkset;
                if linkset.is_nil() {
                    // one script in its window: nothing to cycle
                    return;
                }
                let members: Vec<usize> = self
                    .documents
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.closed && e.linkset == linkset)
                    .map(|(at, _)| at)
                    .collect();
                let Some(at) = members.iter().position(|member| *member == index) else {
                    return;
                };
                let offset = if matches!(step, Some(Step::Previous)) {
                    members.len() - 1
                } else {
                    1
                };
                let target = members[(at + offset) % members.len()];
                self.select_script(linkset, target);
            }
            Some(Step::Split) => {
                let linkset = self.documents[index].linkset;
                if linkset.is_nil() {
                    return;
                }
                if self.splits.remove(&linkset).is_some() {
                    self.split_ratio.remove(&linkset);
                } else {
                    // the next script of the window beside this one
                    let target = self.documents[index].target;
                    let beside = self
                        .documents
                        .iter()
                        .filter(|e| !e.closed && e.linkset == linkset && e.target != target)
                        .map(|e| e.target)
                        .next();
                    if let Some(beside) = beside {
                        self.splits.insert(linkset, (target, beside));
                    }
                }
            }
            Some(Step::Close) => {
                let linkset = self.documents[index].linkset;
                self.documents[index].close_asking();
                // the keyboard goes to the tab that takes its place, so the
                // window commands keep working (a prompt takes it instead)
                if self.documents[index].closed
                    && !linkset.is_nil()
                    && let Some(next) = self
                        .documents
                        .iter()
                        .enumerate()
                        .find(|(at, e)| *at != index && !e.closed && e.linkset == linkset)
                        .map(|(at, _)| at)
                {
                    self.select_script(linkset, next);
                }
            }
            None => {}
        }
    }

    /// Closes the scripts of a window, asking about the text of each one with
    /// unsaved changes: all of them, or all but `keep`.
    fn close_scripts(&mut self, root: Uuid, keep: Option<ScriptTarget>, all: bool) {
        for e in &mut self.documents {
            if e.linkset != root || e.closed || (!all && Some(e.target) == keep) {
                continue;
            }
            e.close_asking();
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        settings: &mut settings::ScriptSettings,
        color_prefs: &mut super::color_picker::Preferences,
        linksets: &[Linkset],
    ) -> Vec<Action> {
        let colors_before = color_prefs.clone();
        let mut actions = Vec::new();
        if self.panels.preferences.show(ctx, p, settings) {
            actions.push(Action::SettingsChanged);
        }
        dialogs::Shortcuts::show(&mut self.panels.shortcuts, ctx, p);
        // The window commands of the script in front, before its source is
        // drawn: Ctrl+Tab and Ctrl+Shift+Tab cycle the scripts of its window,
        // Ctrl+W closes it. Consumed here so the text edit never sees the Tab.
        self.shortcuts(ctx);
        // A Find popup hangs under the menus and toolbar its window drew this
        // frame; the search later in the frame takes that place.
        for e in &mut self.documents {
            e.tools.search_anchor = None;
        }
        // One window per linkset, with its tree, over the scripts whose own
        // windows follow.
        let mut roots: Vec<Uuid> = Vec::new();
        for e in &self.documents {
            if !e.closed && !e.linkset.is_nil() && !roots.contains(&e.linkset) {
                roots.push(e.linkset);
            }
        }
        for root in roots {
            self.show_linkset(ctx, p, root, settings, color_prefs, linksets, &mut actions);
        }
        // a page that chose a language: the script the object made opens
        self.pending_pages(linksets, &mut actions);
        for index in 0..self.documents.len() {
            {
                let e = &mut self.documents[index];
                if e.closed {
                    continue;
                }
                e.recompile_allowed = settings.preprocessor && settings.recompile;
                if e.processor_settings != *settings {
                    e.processor_settings.clone_from(settings);
                }
                e.processor_environment.agent = self.agent;
                e.lua_scripts = self.lua_scripts;
                if e.processor_environment.agent_name != self.agent_name {
                    e.processor_environment.agent_name.clone_from(&self.agent_name);
                }
                // a file job (an external editor's save, an export) answers
                // its own document, even when it is not the one in front
                let body_id = egui::Id::unique((&Self::document_id(e), "source_text"));
                view::poll_files(ctx, e, body_id, settings, &mut actions);
                if let Some(job) = e.processing.as_ref() {
                    for request in job.includes.try_iter() {
                        self.includes.insert(
                            request.id,
                            PendingInclude {
                                owner: e.target,
                                asset: Uuid::nil(),
                                reply: request.reply,
                            },
                        );
                        actions.push(Action::Include {
                            request: request.id,
                            name: request.name,
                        });
                    }
                }
                if let Some(result) = e.processing.as_ref().and_then(|job| job.done.try_recv().ok()) {
                    e.processing = None;
                    self.includes.retain(|_, include| include.owner != e.target);
                    match result {
                        Ok(output) => {
                            e.tools.has_processed = true;
                            e.tools.diagnostics_processed = true;
                            e.tools.processed_source = output.uploaded.clone();
                            // LLPreviewLSL::saveIfNeeded: //mono and //lsl2 only choose between the LSL VMs
                            if let Some(mono) = output.mono
                                && matches!(e.compile, CompileTarget::Mono | CompileTarget::Lsl2)
                            {
                                e.compile = if mono { CompileTarget::Mono } else { CompileTarget::Lsl2 };
                                if let Some(pending) = e.saving.as_mut() {
                                    pending.compile = e.compile;
                                }
                            }
                            e.errors = output.warnings;
                            if let Some(pending) = e.saving.as_ref() {
                                actions.push(Action::Save {
                                    request: pending.request,
                                    target: e.target,
                                    options: ScriptOptions {
                                        compile: pending.compile,
                                        running: pending.running,
                                        experience: pending.experience.unwrap_or_default(),
                                    },
                                    source: output.uploaded,
                                });
                            }
                        }
                        Err(reason) => {
                            e.saving = None;
                            e.close_after_save = false;
                            e.compile_failed = true;
                            e.errors = vec![reason];
                            e.status = aurora_i18n::tr!("script-preproc-failed").into();
                        }
                    }
                }
                // the scripts of a linkset are drawn by their shared window
                if !e.linkset.is_nil() {
                    continue;
                }
                if std::mem::take(&mut e.raise) {
                    super::widgets::expand_floater(ctx, &Self::window_of(e));
                }
            }
            self.show_document(ctx, p, index, settings, color_prefs, &mut actions);
        }
        self.documents.retain(|e| {
            if e.closed
                && let Some(owner) = e.inventory_owner
            {
                crate::ui::inventory::return_focus_to(ctx, owner);
            }
            !e.closed
        });
        // A tab chosen by a key or a click takes the keyboard once its source
        // is drawn: a request made before the text edit draws does not hold.
        for e in &mut self.documents {
            if std::mem::take(&mut e.tools.focus_request) {
                focus::text(ctx, egui::Id::unique((&Self::document_id(e), "source_text")));
            }
        }
        focus::find(ctx, &mut self.documents, &mut self.panels.search);
        // Draw the singleton after every editor has handled its commands. A
        // search opened in a later document must not also draw on the old one.
        let search = &mut self.panels.search;
        let mut bound = false;
        for e in &mut self.documents {
            // the script's own source, in the window it shares with its
            // linkset (the window anchors the Find panel)
            let window = Self::window_of(e);
            let id = egui::Id::unique((&Self::document_id(e), "source_text"));
            if search.owns(id) {
                bound = true;
                let minimized = ctx
                    .data_mut(|d| d.get_persisted::<bool>(egui::Id::unique((&window, "minimized"))))
                    .unwrap_or(false);
                if !minimized && let Some(parent) = ctx.memory(|m| m.area_rect(egui::Id::unique(&window))) {
                    view::search(
                        ctx,
                        p,
                        e,
                        id,
                        search,
                        (egui::Id::unique(&window), parent),
                        settings.search_prefer_left,
                    );
                }
                break;
            }
        }
        if !bound {
            search.open = false;
        }
        if *color_prefs != colors_before {
            actions.push(Action::SettingsChanged);
        }
        actions
    }

    /// The Problems dock's rows of one document: its diagnostics, with the
    /// line and column each one names (a message without one is listed, and
    /// clicking it does nothing).
    fn document_problems(e: &mut Editor) -> Vec<view::Problem> {
        if !e.is_open() {
            return Vec::new();
        }
        let rows: Vec<view::Problem> = e
            .errors
            .iter()
            .map(|message| {
                let (line, column) = editing::error_location(message).unwrap_or((0, 0));
                view::Problem {
                    source: e.name.clone(),
                    line,
                    column,
                    message: message.clone(),
                }
            })
            .collect();
        rows
    }

    /// Save every script the window has open that has something to save
    /// (`linkset` nil: the one script of `target`, as Save). A script the
    /// preprocessor handles answers with its own save later, as Save does.
    pub fn save_all(&mut self, linkset: Uuid, target: ScriptTarget) -> Vec<Action> {
        let mut saves = Vec::new();
        for e in &mut self.documents {
            if e.closed || !e.can_save() {
                continue;
            }
            let mine = if linkset.is_nil() {
                e.target == target
            } else {
                e.linkset == linkset
            };
            if mine && let Some(action) = e.save() {
                saves.push(action);
            }
        }
        saves
    }

    /// One script in its own window: an inventory script, or one whose object
    /// is not linked to others through the editor's own panel.
    fn show_document(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        index: usize,
        settings: &mut settings::ScriptSettings,
        color_prefs: &mut super::color_picker::Preferences,
        actions: &mut Vec<Action>,
    ) {
        let id = Self::window_of(&self.documents[index]);
        let mut open = true;
        let rows = Self::document_problems(&mut self.documents[index]);
        {
            let e = &mut self.documents[index];
            focus::claim(
                ctx,
                egui::LayerId::new(egui::Order::Middle, egui::Id::unique(&id)),
                egui::Id::unique((&Self::document_id(e), "source_text")),
            );
            let title = {
                let name = format!("{}{}", e.name, if e.dirty() { " *" } else { "" });
                aurora_i18n::tr!("script-window-title", name = name.as_str()).to_string()
            };
            let body_id = egui::Id::unique((&Self::document_id(e), "source_text"));
            let panels = &mut self.panels;
            Floater::new(
                &id,
                title,
                ctx.content_rect().center() - Vec2::new(330.0, 260.0),
                Vec2::new(660.0, 520.0),
            )
            .cascade(super::widgets::CascadeGroup::Script)
            .allow_partial_outside()
            .rename_title(e.name.clone(), e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none())
            .show(ctx, p, &mut open, |ui| {
                view::body(ui, p, e, body_id, settings, panels, actions, true, &rows, None, true);
            });
        }
        // Ctrl+click on a name: the declaration, in this script alone
        if let Some(word) = self.documents[index].tools.goto_word.take() {
            let e = &mut self.documents[index];
            let found = navigate::definition(&[(index, e.source.as_str(), e.lang())], &word, index);
            match found {
                Some((_, line, column)) => {
                    let body_id = egui::Id::unique((&Self::document_id(e), "source_text"));
                    view::go_to_line(ctx, e, body_id, line, column);
                }
                None => e.status = aurora_i18n::tr!("script-goto-not-found").into(),
            }
        }
        if let Some(name) = super::widgets::take_title_rename(ctx, &id)
            && self.documents[index].can_edit
            && self.documents[index].loaded
            && self.documents[index].saving.is_none()
            && self.documents[index].loading.is_none()
            && name != self.documents[index].name
        {
            let target = self.documents[index].target;
            self.documents[index].name = name.clone();
            actions.push(Action::Rename { target, name });
        }
        self.document_parts(ctx, p, index, settings, color_prefs, actions, !open);
    }

    /// The dialogs, the colour picker and the close prompt of one document,
    /// and its closing (`close`: its window's X was pressed).
    fn document_parts(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        index: usize,
        settings: &mut settings::ScriptSettings,
        color_prefs: &mut super::color_picker::Preferences,
        actions: &mut Vec<Action>,
        close: bool,
    ) {
        let id = Self::document_id(&self.documents[index]);
        let source_id = egui::Id::unique((&id, "source_text"));
        {
            let e = &mut self.documents[index];
            view::dialogs(ctx, p, e, source_id, settings, &mut self.panels, actions);
            if let Some(literal) = e.colors.clicked.take() {
                e.colors.open(&mut e.source, ctx, source_id, egui::Id::unique(&id), literal);
            }
            let editable = e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none() && !e.tools.processed;
            e.colors.show(ctx, p, color_prefs, &mut e.source, editable);
            if close {
                e.close_asking();
            }
        }
        if !self.documents[index].close_prompt {
            return;
        }
        let e = &mut self.documents[index];
        let mut prompt_open = true;
        let mut cancelled = false;
        Floater::new(
            &format!("{id}_close"),
            aurora_i18n::tr!("ui-inventory-dialogs-save-changes"),
            ctx.content_rect().center() - Vec2::new(220.0, 90.0),
            Vec2::new(440.0, 180.0),
        )
        .fixed()
        .show(ctx, p, &mut prompt_open, |ui| {
            ui.label(aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled_ui(e.can_save(), |ui| flat_button(ui, p, aurora_i18n::tr!("common-save")))
                    .inner
                    .clicked()
                {
                    let action = e.save();
                    if e.saving.is_some() {
                        e.close_after_save = true;
                        e.close_prompt = false;
                        actions.extend(action);
                    }
                }
                if ui
                    .add_enabled_ui(e.saving.is_none(), |ui| flat_button(ui, p, aurora_i18n::tr!("script-discard")))
                    .inner
                    .clicked()
                {
                    e.closed = true;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    e.close_prompt = false;
                    e.close_after_save = false;
                    cancelled = true;
                }
            });
        });
        if !prompt_open {
            e.close_prompt = false;
            e.close_after_save = false;
            cancelled = true;
        }
        if cancelled {
            // a quit held while the scripts asked about their text gives up
            actions.push(Action::CloseCancelled);
        }
    }

    /// The window of one linkset: the tree of its prims' scripts beside the
    /// source. Choosing another script keeps every document, its selection and
    /// its unsaved text.
    /// A page of a new script that chose a language: the first script to
    /// appear in its prim past the ones it listed when it opened is the new
    /// one, and it opens, so the reader lands in the text they chose. The
    /// page stays while nothing appears (an object that refused the create),
    /// and the reader may close it.
    fn pending_pages(&mut self, linksets: &[Linkset], actions: &mut Vec<Action>) {
        let pages: Vec<(ScriptTarget, Uuid, Vec<Uuid>)> = self
            .documents
            .iter()
            .filter_map(|e| {
                let draft = e.draft.as_ref().filter(|draft| draft.chosen)?;
                Some((e.target, draft.object, draft.before.clone()))
            })
            .collect();
        for (target, object, before) in pages {
            let Some(prim) = linksets
                .iter()
                .flat_map(|linkset| linkset.prims.iter())
                .find(|prim| prim.object == object && !prim.loading)
            else {
                continue;
            };
            let Some(script) = new_script_of(&prim.scripts, &before) else {
                continue;
            };
            actions.push(Action::OpenScript(ScriptTarget { object, item: script.item }));
            if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
                e.closed = true;
            }
        }
    }

    fn show_linkset(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        root: Uuid,
        settings: &mut settings::ScriptSettings,
        color_prefs: &mut super::color_picker::Preferences,
        linksets: &[Linkset],
        actions: &mut Vec<Action>,
    ) {
        let window = Self::window_name(root);
        // what the region says of the object's scripts, for the window's bar
        let stats = self.stats_of(root);
        // what a drag of the tree or the tabs carries, for the pane preview
        let tree_drag = egui::Id::unique(("script_tree_drag", root));
        let tab_drag = egui::Id::unique(("script_tab_drag", root));
        let members: Vec<ScriptTarget> = self
            .documents
            .iter()
            .filter(|e| !e.closed && e.linkset == root)
            .map(|e| e.target)
            .collect();
        if members.is_empty() {
            self.active.remove(&root);
            self.tree_selection.remove(&root);
            self.pending_split.remove(&root);
            self.filters.remove(&root);
            return;
        }
        // a script opened or raised from anywhere comes to the front
        let raised = self
            .documents
            .iter()
            .rev()
            .find(|e| !e.closed && e.linkset == root && e.raise)
            .map(|e| e.target);
        if raised.is_some() {
            for e in &mut self.documents {
                if e.linkset == root {
                    e.raise = false;
                }
            }
            super::widgets::expand_floater(ctx, &window);
        }
        // a script being closed asks about its unsaved text before the others
        let prompting = self
            .documents
            .iter()
            .find(|e| !e.closed && e.linkset == root && e.close_prompt)
            .map(|e| e.target);
        let chosen = self.active.get(&root).copied().filter(|target| members.contains(target));
        let active = prompting.or(raised).or(chosen).unwrap_or(members[0]);
        self.active.insert(root, active);
        // the page of a new script is no row of the tree: while it is in front,
        // no row keeps the light -- the script it took the front from stayed
        // selected, and lit beside the new one
        let drafting = self
            .documents
            .iter()
            .any(|e| !e.closed && e.linkset == root && e.target == active && e.draft.is_some());
        // the rows the tree has selected: the ones the window still holds,
        // the script in front alone when there is none
        let selection = {
            let selection = self.tree_selection.entry(root).or_default();
            if drafting {
                selection.clear();
            } else {
                selection.retain(|target| members.contains(target));
                if selection.is_empty() {
                    selection.push(active);
                }
            }
            selection.clone()
        };
        // the panes a window waits for (a script dragged from the tree before
        // it had a document): now that it has one, they take their places
        if let Some((left, right)) = self.pending_split.get(&root).copied()
            && members.contains(&left)
            && members.contains(&right)
        {
            if left != right {
                self.splits.insert(root, (left, right));
            }
            self.pending_split.remove(&root);
        }
        let linkset = linksets.iter().find(|l| l.root == root);
        // a script opened or raised shows in the tree, even inside a shut group
        // of a prim that another prim holds
        if raised.is_some()
            && let Some(linkset) = linkset
        {
            linkset::reveal(ctx, root, linkset, active.object);
        }
        if let Some(prim) = self.reveal.remove(&root)
            && let Some(linkset) = linkset
        {
            linkset::reveal(ctx, root, linkset, prim);
        }
        // a split lasts while both its scripts are open and one of them is the
        // one in front: choosing another script gives a single pane again
        if let Some((left, right)) = self.splits.get(&root).copied()
            && (left == right || !members.contains(&left) || !members.contains(&right) || (active != left && active != right))
        {
            self.splits.remove(&root);
            self.split_ratio.remove(&root);
        }
        let active_index = self.documents.iter().position(|e| e.target == active && !e.closed).unwrap_or(0);
        let title = {
            // the script in front, as the window's title (the tab strip says
            // which one that is)
            let e = &self.documents[active_index];
            let name = format!("{}{}", e.name, if e.dirty() { " *" } else { "" });
            aurora_i18n::tr!("script-window-title", name = name.as_str()).to_string()
        };
        let source_id = egui::Id::unique((&Self::document_id(&self.documents[active_index]), "source_text"));
        // the marks the source draws for a script that did not compile
        let problems = Self::document_problems(&mut self.documents[active_index]);
        // F8 works while a script of this window has the keyboard
        let window_keys = focus::active(ctx).is_some_and(|owner| {
            self.documents
                .iter()
                .any(|e| e.linkset == root && egui::Id::unique((&Self::document_id(e), "source_text")) == owner)
        });
        // Save All: every script this window has open that has something to save
        let save_all = self.documents.iter().any(|e| !e.closed && e.linkset == root && e.can_save());
        let mut open = true;
        let mut select = None;
        // the tree's selection changes and the menu's entries that need the
        // editor, applied after its panel is drawn (the filter is borrowed)
        let mut tree_select: Option<(Vec<ScriptTarget>, Option<ScriptTarget>)> = None;
        let mut tree_open_many: Option<Vec<ScriptTarget>> = None;
        let mut tree_save_many: Option<Vec<ScriptTarget>> = None;
        let mut close_one = None;
        let mut close_others = None;
        let mut close_all = false;
        let mut split_open_at = None;
        let mut dropped = None;
        let mut tree_dropped = None;
        let mut pane_area = None;
        let mut unsplit = false;
        // the prim a new script's page was asked for, from the tree or the strip
        let mut new_draft: Option<Uuid> = None;
        // a fresh window takes the documentation pane's room beside the
        // source's, rather than from it
        let mut window_ui = Floater::new(
            &window,
            title,
            ctx.content_rect().center() - Vec2::new(480.0, 330.0),
            Vec2::new(960.0, 660.0),
        )
        .cascade(super::widgets::CascadeGroup::Script)
        .allow_partial_outside()
        // the menus come right under the title, as a toolbar does
        .compact_header();
        // the tree and a readable source side by side
        window_ui.min_size = Vec2::new(600.0, 380.0);
        // the object the window is about, beside the title that names the
        // script in front
        window_ui.title_note = linkset.map(|linkset| linkset.name.clone()).filter(|name| !name.is_empty());
        // the page a click asked for: opened after the frame, as the window
        // holds the borrows of the editor's own state while it is drawn
        let mut draft_request: Option<(Uuid, String, Vec<Uuid>)> = None;
        window_ui.show(ctx, p, &mut open, |ui| {
            // where Find / Replace hang when the setting keeps them in the
            // window: the menus and the toolbar above the sources
            let bar_top = ui.cursor().top();
            // File, Edit and Help, then the toolbar, once for the window and
            // for the script in front
            let mut chosen = None;
            if let Some(e) = self.documents.get_mut(active_index) {
                view::menus(ui, p, e, source_id, &mut chosen);
            }
            if let Some(command) = chosen
                && let Some(e) = self.documents.get_mut(active_index)
            {
                view::run(command, e, ctx, source_id, settings, &mut self.panels, actions);
            }
            if let Some(e) = self.documents.get_mut(active_index) {
                view::toolbar(ui, p, e, source_id, settings, &mut self.panels, actions, Some(save_all));
            }
            // a popup under them, over the source, for the script that opened
            // Find: no row of the window is taken from the text
            let bar = egui::Rect::from_min_max(
                egui::pos2(ui.max_rect().left(), bar_top),
                egui::pos2(ui.max_rect().right(), ui.cursor().top()),
            );
            for e in self.documents.iter_mut().filter(|e| !e.closed && e.linkset == root) {
                e.tools.search_anchor = settings.search_inline.then_some(bar);
            }
            let gap = ui.spacing().item_spacing.x;
            // the tree keeps the room the reader gave it, or the default
            let room = (ui.available_width() - gap).max(160.0);
            /// Where the tree's edge line stands past the column's right edge:
            /// the middle of the two frames' margins and the gap between them.
            const TREE_EDGE: f32 = 8.0;
            let most = room * 0.45;
            let least = linkset::TREE_MIN.min(most);
            let default_tree = linkset::TREE_WIDTH.clamp(least, most);
            let tree_width = if settings.tree_width == 0 {
                default_tree
            } else {
                (f32::from(settings.tree_width)).clamp(least, most)
            };
            let filter = self.filters.entry(root).or_default();
            // the window's own 3D view, when the app kept one for it
            let stage = self.stages.get_mut(&root);
            // the group's path for each script, the first word of the path
            // under the source: a prim's name (and the prims it is linked
            // under), or the folder's path in the inventory
            let mut paths: std::collections::HashMap<ScriptTarget, String> = std::collections::HashMap::new();
            // the same path for the group alone, for the page of a script that
            // does not exist yet
            let mut prim_paths: std::collections::HashMap<Uuid, String> = std::collections::HashMap::new();
            if let Some(linkset) = linkset {
                let mut names: Vec<&str> = Vec::new();
                for group in &linkset.prims {
                    names.truncate(usize::from(group.depth));
                    names.push(&group.name);
                    let path = names.join(" › ");
                    prim_paths.insert(group.object, path.clone());
                    for script in &group.scripts {
                        paths.insert(
                            ScriptTarget {
                                object: script.object,
                                item: script.item,
                            },
                            path.clone(),
                        );
                    }
                }
            }
            let path_of = |target: ScriptTarget| -> &str { paths.get(&target).map(String::as_str).unwrap_or("") };
            for e in self.documents.iter_mut().filter(|e| !e.closed && e.linkset == root) {
                e.tools.prim = path_of(e.target).to_owned();
            }
            ui.scope(|ui| {
                // The tree's panel, its filter inside it at the top, and the
                // tabs of the open scripts start on the same line: the panel
                // runs down to the source's status bar, the tabs over the
                // source alone.
                ui.spacing_mut().item_spacing.y = 0.0;
                let header = ui.spacing().interact_size.y.max(24.0);
                // the notice of the script in front, over the window's body:
                // the tree and the sources start level under it
                if let Some(e) = self.documents.get_mut(active_index)
                    && e.draft.is_none()
                {
                    view::notice_bar(ui, p, e, actions);
                }
                // the window's status bar takes its room under the columns,
                // and the air the columns keep between them above it; the page
                // of a new script leaves that room empty, so that the tree and
                // the source keep their place from one tab to the next
                let air = ui.spacing().item_spacing.x;
                let bar_room = if self.documents.is_empty() { 0.0 } else { view::STATUS_BAR + air };
                let height = (ui.available_height() - bar_room).max(80.0);
                let columns = ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(tree_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                        match linkset::tree(ui, p, linkset, Some(active), &selection, filter, tree_drag, stage) {
                            Some(linkset::TreeAction::Open(target)) => select = Some(target),
                            Some(linkset::TreeAction::Select { targets, open }) => tree_select = Some((targets, open)),
                            Some(linkset::TreeAction::OpenMany(targets)) => tree_open_many = Some(targets),
                            Some(linkset::TreeAction::TakeMany(targets)) => {
                                for target in targets {
                                    actions.push(Action::TakeScript {
                                        object: target.object,
                                        item: target.item,
                                    });
                                }
                            }
                            Some(linkset::TreeAction::DeleteMany(targets)) => {
                                for target in targets {
                                    actions.push(Action::DeleteScript {
                                        object: target.object,
                                        item: target.item,
                                    });
                                }
                            }
                            Some(linkset::TreeAction::SaveMany(targets)) => tree_save_many = Some(targets),
                            Some(linkset::TreeAction::Close(target)) => close_one = Some(target),
                            Some(linkset::TreeAction::Take(target)) => actions.push(Action::TakeScript {
                                object: target.object,
                                item: target.item,
                            }),
                            Some(linkset::TreeAction::Delete(target)) => actions.push(Action::DeleteScript {
                                object: target.object,
                                item: target.item,
                            }),
                            Some(linkset::TreeAction::Properties(target)) => actions.push(Action::Properties(target)),
                            Some(linkset::TreeAction::SetRunning { target, running }) => {
                                actions.push(Action::Running { target, running });
                            }
                            Some(linkset::TreeAction::NewScript(object)) => new_draft = Some(object),
                            Some(linkset::TreeAction::TakeObject { object, copy }) => {
                                actions.push(Action::TakeObject { object, copy });
                            }
                            Some(linkset::TreeAction::DeleteObject(object)) => actions.push(Action::DeleteObject(object)),
                            Some(linkset::TreeAction::Touch(object)) => actions.push(Action::Touch(object)),
                            Some(linkset::TreeAction::Dropped { target, pos }) => tree_dropped = Some((target, pos)),
                            Some(linkset::TreeAction::Edit(object)) => actions.push(Action::EditObject(object)),
                            Some(linkset::TreeAction::Settings) => {
                                // shown again, a minimized window expands
                                // (LLFloaterReg::showInstance), and the panel is
                                // drawn under the windows: bring it over them
                                super::widgets::open_floater(
                                    ctx,
                                    super::scripts::dialogs::Preferences::WINDOW,
                                    &mut self.panels.preferences.open,
                                );
                                super::widgets::bring_floater_front(ctx, super::scripts::dialogs::Preferences::WINDOW);
                            }
                            None => {}
                        }
                    });
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            // the tabs of the open scripts, across the top of the
                            // source's column: the tree's panel starts at their top,
                            // to the left (the script's own commands are in the
                            // window's toolbar, at the right of its experience)
                            ui.allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), header),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    let mut tabs: Vec<linkset::Tab> = self
                                        .documents
                                        .iter()
                                        .filter(|e| !e.closed && e.linkset == root)
                                        .map(|e| linkset::Tab {
                                            target: e.target,
                                            name: &e.name,
                                            prim: e.draft.as_ref().map_or_else(|| path_of(e.target), |draft| draft.prim.as_str()),
                                            lua: e.lua(),
                                            dirty: e.dirty(),
                                            draft: e.draft.is_some(),
                                        })
                                        .collect();
                                    // in the order the reader dragged them into; the
                                    // rest keep the order they were opened in
                                    if let Some(order) = self.tab_order.get(&root) {
                                        tabs.sort_by_key(|tab| order.iter().position(|target| *target == tab.target).unwrap_or(usize::MAX));
                                    }
                                    match linkset::tabs(
                                        ui,
                                        p,
                                        &tabs,
                                        Some(active),
                                        settings.color(settings::ColorRole::Background, p),
                                        self.splits.get(&root).copied(),
                                        tab_drag,
                                        root != INVENTORY_WINDOW,
                                    ) {
                                        linkset::Tabs::Select(target) => select = Some(target),
                                        linkset::Tabs::Reorder { from, to } => {
                                            let current: Vec<ScriptTarget> = self
                                                .documents
                                                .iter()
                                                .filter(|e| !e.closed && e.linkset == root)
                                                .map(|e| e.target)
                                                .collect();
                                            let order = self.tab_order.entry(root).or_insert_with(|| current.clone());
                                            order.retain(|target| current.contains(target));
                                            reorder_targets(order, from, to);
                                            for target in current {
                                                if !order.contains(&target) {
                                                    order.push(target);
                                                }
                                            }
                                        }
                                        linkset::Tabs::Close(target) => close_one = Some(target),
                                        linkset::Tabs::CloseOthers(target) => close_others = Some(target),
                                        linkset::Tabs::CloseAll => close_all = true,
                                        linkset::Tabs::Split(target) => split_open_at = Some(target),
                                        linkset::Tabs::Dropped { target, pos } => dropped = Some((target, pos)),
                                        linkset::Tabs::Unsplit => unsplit = true,
                                        linkset::Tabs::NewScript => new_draft = Some(active.object),
                                        linkset::Tabs::None => {}
                                    }
                                },
                            );
                            // the tabs sit on their source, as a file tab does
                            if let Some(index) = self.documents.iter().position(|e| e.target == active && !e.closed) {
                                // Ctrl+F belongs to the script in front
                                focus::claim(ctx, egui::LayerId::new(egui::Order::Middle, egui::Id::unique(&window)), source_id);
                                // the two panes, left then right, when the split holds
                                let split = self.splits.get(&root).copied().and_then(|(left, right)| {
                                    let left = self.documents.iter().position(|e| e.target == left && !e.closed)?;
                                    let right = self.documents.iter().position(|e| e.target == right && !e.closed)?;
                                    (left != right).then_some((left, right))
                                });
                                // where a tab let go over the source opens a pane
                                pane_area = Some(ui.available_rect_before_wrap());
                                let ratio = self.split_ratio.entry(root).or_insert(0.5);
                                if let Some((left, right)) = split {
                                    let left_id = egui::Id::unique((&Self::document_id(&self.documents[left]), "source_text"));
                                    let right_id = egui::Id::unique((&Self::document_id(&self.documents[right]), "source_text"));
                                    let side = if left == index { view::Side::Left } else { view::Side::Right };
                                    let (a, b) = two_mut(&mut self.documents, left, right);
                                    let focus = view::split_body(
                                        ui,
                                        p,
                                        a,
                                        left_id,
                                        b,
                                        right_id,
                                        side,
                                        settings,
                                        &mut self.panels,
                                        &problems,
                                        ratio,
                                        stats,
                                        false,
                                    );
                                    if let Some(target) = focus {
                                        self.active.insert(root, target);
                                        self.reveal.insert(root, target.object);
                                    }
                                } else {
                                    let e = &mut self.documents[index];
                                    view::body(
                                        ui,
                                        p,
                                        e,
                                        source_id,
                                        settings,
                                        &mut self.panels,
                                        actions,
                                        false,
                                        &problems,
                                        stats,
                                        false,
                                    );
                                }
                            }
                        },
                    );
                });
                // a tab let go over a pane gives that side the script, or opens
                // the split beside the one in front
                if let Some(area) = pane_area
                    && ctx.input(|i| i.pointer.any_down())
                    && let Some(pos) = ctx.input(|i| i.pointer.interact_pos())
                    && area.contains(pos)
                {
                    let dragging = ctx
                        .data(|d| d.get_temp::<Option<(ScriptTarget, String)>>(tree_drag))
                        .flatten()
                        .map(|(target, _)| target)
                        .or_else(|| ctx.data(|d| d.get_temp::<Option<ScriptTarget>>(tab_drag)).flatten());
                    if let Some(target) = dragging
                        && target != active
                    {
                        let ratio = self.split_ratio.get(&root).copied().unwrap_or(0.5);
                        let boundary = if self.splits.contains_key(&root) {
                            area.left() + view::split_widths(area.width(), ratio).0
                        } else {
                            area.center().x
                        };
                        let (half, at) = if pos.x < boundary {
                            (egui::Rect::from_min_max(area.min, egui::pos2(boundary, area.max.y)), boundary)
                        } else {
                            (egui::Rect::from_min_max(egui::pos2(boundary, area.min.y), area.max), boundary)
                        };
                        ui.painter()
                            .rect_filled(half, egui::CornerRadius::ZERO, p.violet.gamma_multiply(0.12));
                        ui.painter().vline(at, area.y_range(), egui::Stroke::new(2.0, p.violet));
                    }
                }
                if let Some((target, pos)) = dropped.or(tree_dropped)
                    && let Some(area) = pane_area
                    && area.contains(pos)
                    && target != active
                {
                    let ratio = self.split_ratio.get(&root).copied().unwrap_or(0.5);
                    let boundary = if self.splits.contains_key(&root) {
                        area.left() + view::split_widths(area.width(), ratio).0
                    } else {
                        area.center().x
                    };
                    let left = pos.x < boundary;
                    let (left_pane, right_pane) = self.splits.get(&root).copied().unwrap_or((active, active));
                    let pair = if left { (target, right_pane) } else { (left_pane, target) };
                    if self.documents.iter().any(|e| !e.closed && e.target == target) {
                        if pair.0 != pair.1 {
                            self.splits.insert(root, pair);
                        }
                    } else {
                        // the app opens the script; the split waits for it
                        self.pending_split.insert(root, pair);
                        actions.push(Action::OpenScript(target));
                    }
                }
                // the status bar of the script in front, across the window:
                // the caret's place at the left, the script's weight at the right
                if let Some(e) = self.documents.get_mut(active_index) {
                    ui.add_space(air);
                    if let Some(change) = view::status_bar(ui, p, e, source_id, settings, stats) {
                        actions.push(Action::BarChange(change));
                    }
                }
                // the documentation pane's edge, dragged to give it more or
                // less room; a double click takes the default back
                // the tree's edge, dragged to give the list more or less room;
                // a double click takes the default back
                if let Some(width) = super::widgets::resize_side(
                    ui,
                    p,
                    columns.response.rect,
                    tree_width,
                    // the room between the tree's face and the source's, the
                    // two frames' margins and the gap between the columns
                    TREE_EDGE,
                    default_tree,
                    least,
                    most,
                    egui::Id::unique(("script_tree_resize", root)),
                ) {
                    // the default is no choice at all: it follows the window
                    settings.tree_width = if (width - default_tree).abs() < 0.5 {
                        0
                    } else {
                        width.round() as u16
                    };
                }
                // the page of a new script, asked for by the tree or the plus:
                // it opens after this frame, its tab taking the front
                if let Some(object) = new_draft.take()
                    && !object.is_nil()
                {
                    let prim = prim_paths.get(&object).cloned().unwrap_or_default();
                    let before = linkset
                        .and_then(|linkset| linkset.prims.iter().find(|prim| prim.object == object))
                        .map(|prim| prim.scripts.iter().map(|script| script.item).collect())
                        .unwrap_or_default();
                    draft_request = Some((object, prim, before));
                }
            });
        });
        if let Some((object, prim, before)) = draft_request {
            self.open_draft(root, object, prim, if self.lua_scripts { Lang::Lua } else { Lang::Lsl }, before);
            ctx.request_repaint();
        }
        // F8 and Shift+F8, from the script in front and its caret: the
        // diagnostics of the window in turn
        if window_keys
            && let Some(next) = view::problem_key(ctx)
            && !problems.is_empty()
        {
            let e = &self.documents[active_index];
            let at = (active_index, e.tools.position.0 + e.first_line());
            let rows: Vec<(usize, usize)> = problems.iter().map(|problem| (active_index, problem.line)).collect();
            if let Some(row) = view::problem_row(&rows, at, next)
                && let Some(problem) = problems.get(row)
            {
                let e = &mut self.documents[active_index];
                let body_id = egui::Id::unique((&Self::document_id(e), "source_text"));
                view::go_to_error(ctx, e, body_id, problem.line, problem.column);
            }
        }
        // Ctrl+click on a name (or the menu's command): the window takes the
        // caret to where the name is declared
        if self.documents[active_index].tools.goto_word.is_some() {
            let word = self.documents[active_index].tools.goto_word.take().unwrap_or_default();
            let scripts: Vec<(usize, &str, Lang)> = self
                .documents
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.closed && e.linkset == root)
                .map(|(index, e)| (index, e.source.as_str(), e.lang()))
                .collect();
            let found = navigate::definition(&scripts, &word, active_index);
            match found {
                Some((document, line, column)) => {
                    let e = &mut self.documents[document];
                    self.active.insert(root, e.target);
                    let body_id = egui::Id::unique((&Self::document_id(e), "source_text"));
                    view::go_to_line(ctx, e, body_id, line, column);
                }
                None => {
                    self.documents[active_index].status = aurora_i18n::tr!("script-goto-not-found").into();
                }
            }
        }
        // the tree's selection and the entries that need the editor, once
        // its panel has been drawn
        if let Some((targets, open)) = tree_select {
            self.tree_selection.insert(root, targets);
            if let Some(target) = open {
                select = Some(target);
            }
        }
        if let Some(targets) = tree_open_many {
            for target in targets {
                match self.documents.iter().position(|e| e.target == target && !e.closed) {
                    Some(index) => self.select_script(root, index),
                    None => actions.push(Action::OpenScript(target)),
                }
            }
        }
        if let Some(targets) = tree_save_many {
            for target in targets {
                if let Some(index) = self.documents.iter().position(|e| e.target == target && !e.closed && e.can_save())
                    && let Some(action) = self.documents[index].save()
                {
                    actions.push(action);
                }
            }
        }
        if let Some(target) = select {
            ctx.request_repaint();
            match self.documents.iter().position(|e| e.target == target && !e.closed) {
                Some(index) => self.select_script(root, index),
                // that script has no editor yet: the app gives it one, with
                // the rights and the source of its item
                None => actions.push(Action::OpenScript(target)),
            }
        }
        if let Some(target) = close_one
            && let Some(index) = self.documents.iter().position(|e| e.target == target && !e.closed)
        {
            self.documents[index].close_asking();
        }
        // the tab's menu: another script beside this window's, or one at a time
        if let Some(target) = split_open_at
            && target != active
        {
            self.splits.insert(root, (active, target));
        }
        if unsplit {
            self.splits.remove(&root);
            self.split_ratio.remove(&root);
        }
        // the tab's menu: the others, or every script of the window
        if let Some(target) = close_others {
            self.close_scripts(root, Some(target), false);
        }
        if close_all {
            self.close_scripts(root, None, true);
        }
        // the window's X closes every script of the linkset, asking about
        // each one that has unsaved text
        if !open {
            self.close_scripts(root, None, true);
        }
        let active = self.active.get(&root).copied().unwrap_or(active);
        if let Some(index) = self.documents.iter().position(|e| e.target == active && !e.closed) {
            self.document_parts(ctx, p, index, settings, color_prefs, actions, false);
        }
    }

    /// A script chosen in the tree: the keyboard goes to its source, at the
    /// cursor it kept, and the colour picker and Find of the former script
    /// close.
    fn select_script(&mut self, root: Uuid, index: usize) {
        let target = self.documents[index].target;
        // the tree follows the script chosen: its row alone is selected
        self.tree_selection.insert(root, vec![target]);
        // the chosen script's prim opens in the tree, so the list shows it
        self.reveal.insert(root, target.object);
        // a script that is not one of the split's two panes gives a single
        // pane again, now rather than at the next frame's check
        if let Some((left, right)) = self.splits.get(&root).copied()
            && target != left
            && target != right
        {
            self.splits.remove(&root);
            self.split_ratio.remove(&root);
        }
        if let Some(previous) = self.active.insert(root, target)
            && previous != target
        {
            if let Some(e) = self.documents.iter_mut().find(|e| e.target == previous) {
                e.colors.cancel(&mut e.source);
            }
            // Find belongs to the document it was opened in: it would search
            // the text of a script the user is no longer looking at
            self.panels.search.close();
        }
        let e = &mut self.documents[index];
        if e.loaded {
            e.tools.focus_start = false;
            // after the sources of the frame are drawn, as a click's would be
            e.tools.focus_request = true;
        } else {
            e.tools.focus_start = true;
        }
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
