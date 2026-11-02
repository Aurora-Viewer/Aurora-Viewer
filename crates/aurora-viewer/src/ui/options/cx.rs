//! The engine of the preferences' pages. Every setting is one row declared
//! through [`Cx`]: the same declaration lays the row out in the page's two
//! columns, marks it when it differs from its default (a dot that puts it
//! back), finds it for the search of the window, lists it among the
//! modified settings, and resets it with its page. A row cannot be shown
//! without being searchable and resettable, nor the other way round.
//!
//! The rows of a page run in several modes: drawn (`Show`), drawn when they
//! match the search or differ from their default (`Search`, `Modified`, all
//! pages at once), counted for the sidebar's chips (`Count`), put back to
//! their default (`Reset`), or listed for the tests (`Collect`).

use super::{OptionsResult, OptionsUi, Page};
use crate::settings::Settings;
use crate::theme::Palette;
use egui::{Rect, Sense, Shape, Vec2, pos2, vec2};

/// What the rows do in this pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    /// The page shown: every row of its tab.
    Show,
    /// The rows matching the search, from every page.
    Search,
    /// The rows that differ from their default, from every page.
    Modified,
    /// Nothing drawn: the modified rows and the search hits of each page.
    Count,
    /// Nothing drawn: every row back to its default.
    Reset,
    /// Nothing drawn: the settings each row edits (tests).
    #[cfg(test)]
    Collect,
}

impl Mode {
    fn draws(self) -> bool {
        matches!(self, Mode::Show | Mode::Search | Mode::Modified)
    }
}

/// The settings a row edits: whether they differ from their defaults, and
/// how to put them back.
pub(super) trait Track {
    fn modified(&self, s: &Settings, d: &Settings) -> bool;
    fn reset(&self, s: &mut Settings, d: &Settings);
    /// The settings' paths (`lookat.private`), for the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    fn paths(&self, out: &mut Vec<&'static str>);
}

/// One setting, reached by its path (see [`field!`]).
pub(super) struct F<T: 'static> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub path: &'static str,
    pub get: fn(&Settings) -> &T,
    pub get_mut: fn(&mut Settings) -> &mut T,
}

impl<T: PartialEq + Clone> Track for F<T> {
    fn modified(&self, s: &Settings, d: &Settings) -> bool {
        (self.get)(s) != (self.get)(d)
    }
    fn reset(&self, s: &mut Settings, d: &Settings) {
        *(self.get_mut)(s) = (self.get)(d).clone();
    }
    fn paths(&self, out: &mut Vec<&'static str>) {
        out.push(self.path);
    }
}

/// Several settings edited by one row.
pub(super) struct Many<'x>(pub &'x [&'x dyn Track]);

impl Track for Many<'_> {
    fn modified(&self, s: &Settings, d: &Settings) -> bool {
        self.0.iter().any(|t| t.modified(s, d))
    }
    fn reset(&self, s: &mut Settings, d: &Settings) {
        self.0.iter().for_each(|t| t.reset(s, d));
    }
    fn paths(&self, out: &mut Vec<&'static str>) {
        self.0.iter().for_each(|t| t.paths(out));
    }
}

/// A row that edits no saved setting (a state of the grid, a button, a
/// preview): never modified, nothing to reset.
pub(super) struct Nothing;

impl Track for Nothing {
    fn modified(&self, _: &Settings, _: &Settings) -> bool {
        false
    }
    fn reset(&self, _: &mut Settings, _: &Settings) {}
    fn paths(&self, _: &mut Vec<&'static str>) {}
}

/// A row's own way of telling and resetting its settings (a shortcut, one
/// interface sound).
pub(super) struct Custom<M: Fn(&Settings, &Settings) -> bool, R: Fn(&mut Settings, &Settings)> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub path: &'static str,
    pub modified: M,
    pub reset: R,
}

impl<M: Fn(&Settings, &Settings) -> bool, R: Fn(&mut Settings, &Settings)> Track for Custom<M, R> {
    fn modified(&self, s: &Settings, d: &Settings) -> bool {
        (self.modified)(s, d)
    }
    fn reset(&self, s: &mut Settings, d: &Settings) {
        (self.reset)(s, d)
    }
    fn paths(&self, out: &mut Vec<&'static str>) {
        out.push(self.path);
    }
}

/// `field!(lookat.private)`: the [`F`] of a setting.
macro_rules! field {
    ($($f:ident).+) => {
        $crate::ui::options::cx::F {
            path: stringify!($($f).+),
            get: |s: &$crate::settings::Settings| &s.$($f).+,
            get_mut: |s: &mut $crate::settings::Settings| &mut s.$($f).+,
        }
    };
}

/// `fields!(a, b.c)`: the settings of a row that edits several.
macro_rules! fields {
    ($($($f:ident).+),+ $(,)?) => {
        $crate::ui::options::cx::Many(&[ $( &field!($($f).+) as &dyn $crate::ui::options::cx::Track ),+ ])
    };
}

pub(super) use {field, fields};

/// Folds a text for the search: lower case, without accents (é → e, œ → oe),
/// as people type a setting's name.
pub(super) fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => out.push('a'),
            'ç' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' => out.push('i'),
            'ñ' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            'ß' => out.push_str("ss"),
            '’' => out.push('\''),
            _ => out.push(c),
        }
    }
    out
}

/// The words of a text, folded: its runs of letters and digits ("Ctrl+M"
/// is "ctrl" and "m").
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// The words of a search, folded.
pub(super) fn words(query: &str) -> Vec<String> {
    tokens(&fold(query)).map(str::to_owned).collect()
}

/// Whether every word of the search starts a word of `text`, as a name is
/// typed ("ombre" finds "Ombres", not "Nombre").
pub(super) fn found(text: &str, words: &[String]) -> bool {
    let folded = fold(text);
    words.iter().all(|w| tokens(&folded).any(|t| t.starts_with(w.as_str())))
}

/// Where the first word of the search starts a word of `label`, and where
/// it ends (bytes), to light it: folding keeps one character for one,
/// except for the ligatures, where the light simply stays off.
fn found_in(label: &str, words: &[String]) -> Option<(usize, usize)> {
    let folded = fold(label);
    if folded.chars().count() != label.chars().count() {
        return None;
    }
    let chars: Vec<char> = folded.chars().collect();
    words.iter().find_map(|w| {
        let w: Vec<char> = w.chars().collect();
        let start = (0..chars.len())
            .find(|&i| (i == 0 || !chars[i - 1].is_alphanumeric()) && chars.get(i..i + w.len()).is_some_and(|s| s == w.as_slice()))?;
        let byte = |n: usize| label.char_indices().nth(n).map_or(label.len(), |(i, _)| i);
        Some((byte(start), byte(start + w.len())))
    })
}

/// Per page: the modified rows and the search hits (`Count`).
#[derive(Default, Clone)]
pub(super) struct Counts {
    pub modified: Vec<(Page, usize)>,
    pub hits: Vec<(Page, usize)>,
}

impl Counts {
    fn add(list: &mut Vec<(Page, usize)>, page: Page) {
        match list.iter_mut().find(|(p, _)| *p == page) {
            Some((_, n)) => *n += 1,
            None => list.push((page, 1)),
        }
    }
    pub fn modified(&self, page: Page) -> usize {
        self.modified.iter().find(|(p, _)| *p == page).map_or(0, |(_, n)| *n)
    }
    pub fn hits(&self, page: Page) -> usize {
        self.hits.iter().find(|(p, _)| *p == page).map_or(0, |(_, n)| *n)
    }
    pub fn total_modified(&self) -> usize {
        self.modified.iter().map(|(_, n)| n).sum()
    }
}

/// The row whose cog holds the rows being declared.
struct Parent {
    label: String,
    off: bool,
    id: egui::Id,
}

/// What the next row is (set by the calls just before it).
#[derive(Default)]
struct Next {
    experimental: bool,
    wide: bool,
    below: bool,
    /// The enclosing group already names this full-width control.
    section_body: bool,
    /// Its `subs` always show under it, without a cog.
    unfolded: bool,
    /// Not greyed behind a cog that is off: another setting uses it too.
    live: bool,
    keywords: String,
}

/// Room between the two columns, and the width under which there is one.
const GAP: f32 = 24.0;
const TWO_COLUMNS: f32 = 660.0;
/// The gutter of the modified dot, left of the label.
const GUTTER: f32 = 18.0;
/// Indent of the rows under a cog.
const SUB_INDENT: f32 = 18.0;
/// Width of a row's cog.
const COG_W: f32 = 40.0;
const ROW_MIN_H: f32 = 34.0;

/// The rows laid out so far: the current line of the two columns.
#[derive(Default)]
struct Grid {
    left: f32,
    width: f32,
    /// The column the next row takes, and whether a line is under way.
    col: usize,
    open: bool,
    top: f32,
    height: f32,
    indent: f32,
}

pub(super) struct Cx<'a> {
    pub ui: &'a mut egui::Ui,
    pub p: Palette,
    pub s: &'a mut Settings,
    pub st: &'a mut OptionsUi,
    pub r: &'a mut OptionsResult,
    d: &'a Settings,
    mode: Mode,
    number_width: f32,
    words: Vec<String>,
    page: Page,
    tab: usize,
    group: String,
    /// The search or modified view still has to title the current group.
    group_untitled: bool,
    /// The last row declared: the parent of the `subs` that follow, and
    /// whether they show without a cog (`unfolded`).
    last: Option<(egui::Id, String, bool)>,
    /// Inside a parent's `subs`: its label (shown before the row in the
    /// search) and whether it is off (its rows greyed).
    parent: Option<Parent>,
    next: Next,
    grid: Grid,
    /// The background of an open cog's block: its top and the shape kept for it.
    block: Option<(f32, egui::layers::ShapeIdx)>,
    pub counts: Counts,
    /// Rows declared so far (the number on a cog).
    rows_seen: usize,
    /// The rows of the group are greyed (`greyed`).
    greyed: bool,
    /// Another page's rows drawn here too (`borrow`): the settings they
    /// edit; the page's other rows, groups and notes are skipped.
    pick: Option<Vec<&'static str>>,
    #[cfg(test)]
    pub collected: Vec<(Page, &'static str)>,
    /// A setting changed (the app saves and applies them).
    pub changed: bool,
}

impl<'a> Cx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ui: &'a mut egui::Ui,
        p: Palette,
        s: &'a mut Settings,
        d: &'a Settings,
        st: &'a mut OptionsUi,
        r: &'a mut OptionsResult,
        mode: Mode,
        query: &str,
    ) -> Self {
        // DragValue uses interact_size.x for its button AND text editor.
        // Reserve its slot before measuring the right-aligned controls:
        // changing digits, units or a named value must not move the rail.
        let number_width = if mode.draws() {
            let font = ui.style().drag_value_text_style.resolve(ui.style());
            [
                i32::MIN.to_string(),
                format!("65536{}", aurora_i18n::tr!("bandwidth-kbps-suffix")),
                aurora_i18n::tr!("ui-options-away-disabled").to_owned(),
                aurora_i18n::tr!("ui-options-unlimited").to_owned(),
            ]
            .into_iter()
            .map(|text| ui.painter().layout_no_wrap(text, font.clone(), p.ink).size().x)
            .fold(0.0, f32::max)
                + 2.0 * ui.spacing().button_padding.x
        } else {
            0.0
        };
        Cx {
            ui,
            p,
            s,
            st,
            r,
            d,
            mode,
            number_width,
            words: words(query),
            page: Page::Aurora,
            tab: 0,
            group: String::new(),
            group_untitled: false,
            last: None,
            parent: None,
            next: Next::default(),
            grid: Grid::default(),
            block: None,
            counts: Counts::default(),
            rows_seen: 0,
            greyed: false,
            pick: None,
            #[cfg(test)]
            collected: Vec::new(),
            changed: false,
        }
    }

    /// The rows that follow belong to this page and tab.
    pub fn enter(&mut self, page: Page, tab: usize) {
        self.page = page;
        self.tab = tab;
    }

    /// The next row carries the experimental mark.
    pub fn experimental(&mut self) -> &mut Self {
        self.next.experimental = true;
        self
    }

    /// The next row takes both columns (a wide control).
    pub fn wide(&mut self) -> &mut Self {
        self.next.wide = true;
        self
    }

    /// The next row takes both columns, its control under its label (a
    /// text, a list).
    pub fn below(&mut self) -> &mut Self {
        self.next.wide = true;
        self.next.below = true;
        self
    }

    /// A group's single control: keep its label for search and navigation,
    /// without repeating the section heading on the normal page.
    pub fn section_body(&mut self) -> &mut Self {
        self.next.section_body = true;
        self.below()
    }

    /// The `subs` of the next row always show under it, as behind an open
    /// cog, and it has no cog: for the one value its switch is turned on
    /// to set (a frame rate limit).
    pub fn unfolded(&mut self) -> &mut Self {
        self.next.unfolded = true;
        self
    }

    /// The next row stays live behind a cog that is off, when `on`: another
    /// setting that is on uses its value too (the friends' color, which
    /// also tints the friends' plates).
    pub fn live(&mut self, on: bool) -> &mut Self {
        self.next.live = on;
        self
    }

    /// The rows that follow, to the end of their group, are greyed (what
    /// they set means nothing in the state of another setting).
    pub fn greyed(&mut self, off: bool) {
        self.greyed = off;
    }

    /// Hide an inactive branch on its page, while retaining its settings
    /// in search, modified views, reset and declaration passes.
    pub fn conditional(&mut self, visible: bool, body: impl FnOnce(&mut Self)) {
        if self.mode != Mode::Show || visible {
            let before = self.greyed;
            self.greyed |= !visible;
            body(self);
            self.greyed = before;
        }
    }

    /// A search is under way: the rows' `keywords` are read (a page skips
    /// building costly ones otherwise).
    pub fn searching(&self) -> bool {
        !self.words.is_empty()
    }

    /// More words the search finds the next row by (the names of its
    /// choices, the keys of a shortcut).
    pub fn keywords(&mut self, words: &str) -> &mut Self {
        self.next.keywords.push(' ');
        self.next.keywords.push_str(words);
        self
    }

    /// The rows of another page's tab that edit `paths`, drawn here as well
    /// (Accessibility gathers settings that live on other pages). They are
    /// that page's own declarations, so the same setting, help, modified
    /// dot and control; only drawn: the search, the modified view, the
    /// counts and the resets reach them on their page, a setting keeping
    /// one page. They show in the order of that page, without its cogs.
    pub fn borrow(&mut self, page: Page, tab: usize, paths: &[&'static str]) {
        if self.mode != Mode::Show || self.pick.is_some() {
            return;
        }
        self.pick = Some(paths.to_vec());
        super::pages::draw(self, page, tab);
        self.pick = None;
        self.last = None;
        self.parent = None;
    }

    /// A titled group of rows: a small-caps title and its line, then the
    /// rows in two columns.
    pub fn group(&mut self, title: &str, body: impl FnOnce(&mut Self)) {
        if self.pick.is_some() {
            // a borrowed row joins the group it is shown in
            let before = std::mem::replace(&mut self.group, title.to_owned());
            body(self);
            self.group = before;
            self.greyed = false;
            self.last = None;
            return;
        }
        self.end_block();
        self.end_line();
        let before = std::mem::replace(&mut self.group, title.to_owned());
        if self.mode == Mode::Show {
            self.group_title(title);
        }
        self.group_untitled = matches!(self.mode, Mode::Search | Mode::Modified);
        body(self);
        self.end_block();
        self.end_line();
        if self.mode == Mode::Show || (self.mode.draws() && !self.group_untitled) {
            self.ui.add_space(14.0);
        }
        self.group = before;
        self.group_untitled = false;
        self.greyed = false;
        self.last = None;
    }

    /// A line of text under the group's title (shown on the page only).
    pub fn note(&mut self, text: &str) {
        if self.mode != Mode::Show || self.pick.is_some() {
            return;
        }
        self.end_block();
        self.end_line();
        let x = self.ui.available_rect_before_wrap().left() + GUTTER + self.grid.indent;
        self.ui.horizontal(|ui| {
            ui.add_space(x - ui.cursor().left());
            ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(self.p.muted)).wrap());
        });
        self.ui.add_space(4.0);
    }

    /// A framed notice at the top of an experimental page: the experimental
    /// mark, then `text`, on the surface of an open cog's block (shown on
    /// the page only).
    pub fn experimental_notice(&mut self, text: &str) {
        if self.mode != Mode::Show || self.pick.is_some() {
            return;
        }
        self.end_block();
        self.end_line();
        const PAD: f32 = 12.0;
        const MARK: f32 = 16.0;
        let p = self.p;
        let width = self.ui.available_width();
        let galley = self.ui.painter().layout(
            text.to_owned(),
            egui::FontId::proportional(12.5),
            p.ink,
            (width - 3.0 * PAD - MARK).max(1.0),
        );
        let height = galley.size().y.max(MARK) + 2.0 * PAD;
        let (rect, _) = self.ui.allocate_exact_size(vec2(width, height), Sense::hover());
        self.ui.painter().rect_filled(rect, self.ui.visuals().window_corner_radius, p.field);
        let mark = Rect::from_min_size(pos2(rect.left() + PAD, rect.top() + PAD), Vec2::splat(MARK));
        crate::ui::widgets::experimental_mark(self.ui, &p, mark, egui::Id::unique(("preferences-notice", self.page)));
        let text_top = rect.top() + PAD + (MARK - galley.size().y).max(0.0) / 2.0;
        self.ui.painter().galley(pos2(mark.right() + PAD, text_top), galley, p.ink);
        self.ui.add_space(14.0);
    }

    /// Settings the page resets without a row of their own (they follow
    /// another row, or another window edits them).
    pub fn track(&mut self, t: &dyn Track) {
        match self.mode {
            Mode::Reset if !self.rlva_locked(t) => t.reset(self.s, self.d),
            #[cfg(test)]
            Mode::Collect => {
                let mut paths = Vec::new();
                t.paths(&mut paths);
                self.collected.extend(paths.into_iter().map(|p| (self.page, p)));
            }
            _ => {}
        }
    }

    fn rlva_locked(&self, track: &dyn Track) -> bool {
        if !self.st.rlva_camera_preset_locked && !self.st.rlva_camera_fov_locked {
            return false;
        }
        let mut paths = Vec::new();
        track.paths(&mut paths);
        paths.iter().any(|path| match *path {
            "camera.preset" | "camera.camera_offset" | "camera.focus_offset" | "camera.offset_scale" => self.st.rlva_camera_preset_locked,
            "camera.fov" => self.st.rlva_camera_fov_locked,
            _ => false,
        })
    }

    /// A row: `label` on the left (`hint` under the pointer), the control
    /// `body` draws on the right; `t` the settings it edits. True when the
    /// body reports a change.
    pub fn row(&mut self, label: &str, hint: &str, t: &dyn Track, body: impl FnOnce(&mut egui::Ui, &mut Settings) -> bool) -> bool {
        self.row_with(label, hint, t, |ui, s, _, _| body(ui, s))
    }

    /// `row` whose control also needs the window's state or its result.
    pub fn row_with(
        &mut self,
        label: &str,
        hint: &str,
        t: &dyn Track,
        body: impl FnOnce(&mut egui::Ui, &mut Settings, &mut OptionsUi, &mut OptionsResult) -> bool,
    ) -> bool {
        let next = std::mem::take(&mut self.next);
        self.rows_seen += 1;
        let id = self.row_id(label);
        self.last = Some((id, label.to_owned(), next.unfolded));
        if let Some(pick) = &self.pick {
            let mut paths = Vec::new();
            t.paths(&mut paths);
            if !paths.iter().any(|p| pick.contains(p)) {
                return false;
            }
        }
        let modified = t.modified(self.s, self.d);
        match self.mode {
            Mode::Reset => {
                if !self.rlva_locked(t) {
                    t.reset(self.s, self.d);
                }
                return false;
            }
            #[cfg(test)]
            Mode::Collect => {
                let mut paths = Vec::new();
                t.paths(&mut paths);
                self.collected.extend(paths.into_iter().map(|p| (self.page, p)));
                return false;
            }
            Mode::Count => {
                if modified {
                    Counts::add(&mut self.counts.modified, self.page);
                }
                if !self.words.is_empty() && self.matches(label, hint, &next.keywords) {
                    Counts::add(&mut self.counts.hits, self.page);
                }
                // a row asked by its label behind a closed cog: open it
                if let Some(parent) = &self.parent
                    && self
                        .st
                        .reveal
                        .is_some_and(|(page, tab, l)| page == self.page && tab == self.tab && l == label)
                {
                    self.st.open_subs.insert(parent.id);
                }
                return false;
            }
            Mode::Search if !self.matches(label, hint, &next.keywords) => return false,
            Mode::Modified if !modified => return false,
            _ => {}
        }
        if self.mode == Mode::Show
            && self
                .st
                .reveal
                .is_some_and(|(page, tab, l)| page == self.page && tab == self.tab && l == label)
        {
            self.st.reveal = None;
            self.st.flash = Some((id, self.ui.input(|i| i.time)));
            self.st.flash_scroll = true;
        }
        if self.group_untitled {
            self.end_line();
            self.crumb();
            self.group_untitled = false;
        }
        let changed = self.draw_row(id, label, hint, t, modified, &next, body);
        self.changed |= changed;
        changed
    }

    /// The rows behind the cog of the row just declared. `off`: that row
    /// is off, they are greyed and its cog dimmed. On the page they show
    /// while its cog is open; the search, the modified view and the resets
    /// always go through them. Under an `unfolded` row they always show.
    pub fn subs(&mut self, off: bool, body: impl FnOnce(&mut Self)) {
        let Some((id, label, unfolded)) = self.last.take() else {
            body(self);
            return;
        };
        let before = self.parent.replace(Parent { label, off, id });
        match self.mode {
            // borrowed: a row behind a cog shows on its own, greyed while
            // its parent is off
            Mode::Show if self.pick.is_some() => body(self),
            Mode::Count if unfolded => body(self),
            Mode::Count => {
                // the number shown on the cog, and whether to dim it (the
                // hidden rows still count for their page)
                let rows = self.rows_seen;
                body(self);
                self.st.subs.insert(id, (self.rows_seen - rows, off));
                if self.st.demo_open_cogs {
                    self.st.open_subs.insert(id);
                }
            }
            Mode::Show => {
                if unfolded || self.st.open_subs.contains(&id) {
                    // greyed one by one while the parent is off (`draw_row`)
                    let indent = self.grid.indent;
                    self.grid.indent += SUB_INDENT;
                    body(self);
                    self.end_line();
                    self.end_block();
                    self.grid.indent = indent;
                    if unfolded {
                        // two in a row read as two blocks, not one
                        self.ui.add_space(6.0);
                    }
                }
            }
            _ => body(self),
        }
        self.parent = before;
        self.last = None;
    }

    /// A switch.
    pub fn toggle(&mut self, label: &str, hint: &str, f: &F<bool>) -> bool {
        let (p, get) = (self.p, f.get_mut);
        self.row(label, hint, f, |ui, s| super::toggle(ui, &p, get(s)))
    }

    /// A slider of the setting's own values.
    pub fn slider<T: egui::emath::Numeric + PartialEq + Clone>(
        &mut self,
        label: &str,
        hint: &str,
        f: &F<T>,
        range: std::ops::RangeInclusive<T>,
        suffix: &str,
    ) -> bool {
        let get = f.get_mut;
        self.row(label, hint, f, |ui, s| super::slider(ui, get(s), range, suffix))
    }

    /// A slider of whole values.
    pub fn slider_whole(&mut self, label: &str, hint: &str, f: &F<f32>, range: std::ops::RangeInclusive<f32>, suffix: &str) -> bool {
        let get = f.get_mut;
        self.row(label, hint, f, |ui, s| super::slider_whole(ui, get(s), range, suffix))
    }

    /// A slider of named levels.
    pub fn stepped(&mut self, label: &str, hint: &str, f: &F<u8>, levels: &[&str]) -> bool {
        let (p, get) = (self.p, f.get_mut);
        self.keywords(&levels.join(" "));
        self.row(label, hint, f, |ui, s| super::stepped(ui, &p, get(s), levels))
    }

    /// A drop-down list; its choices are found by the search too.
    pub fn combo<T: PartialEq + Copy + Clone>(&mut self, label: &str, hint: &str, f: &F<T>, choices: &[(T, &str)]) -> bool {
        let get = f.get_mut;
        let words: Vec<&str> = choices.iter().map(|(_, l)| *l).collect();
        self.keywords(&words.join(" "));
        self.row(label, hint, f, |ui, s| super::combo(ui, get(s), choices))
    }

    /// A drop-down list whose choices with a reason are greyed, the reason
    /// on hover.
    pub fn combo_blocked<T: PartialEq + Copy + Clone>(
        &mut self,
        label: &str,
        hint: &str,
        f: &F<T>,
        choices: &[(T, &str, Option<&str>)],
    ) -> bool {
        let get = f.get_mut;
        let words: Vec<&str> = choices.iter().map(|(_, l, _)| *l).collect();
        self.keywords(&words.join(" "));
        self.row(label, hint, f, |ui, s| super::combo_blocked(ui, get(s), choices))
    }

    /// A few choices side by side (`accent`: in Aurora's accent); a long
    /// one takes both columns.
    pub fn choice<T: PartialEq + Copy + Clone>(&mut self, label: &str, hint: &str, f: &F<T>, choices: &[(T, &str, bool)]) -> bool {
        let (p, get) = (self.p, f.get_mut);
        let words: Vec<&str> = choices.iter().map(|(_, l, _)| *l).collect();
        self.keywords(&words.join(" "));
        if super::segmented_width(self.ui, choices) > 230.0 {
            self.wide();
        }
        self.row(label, hint, f, |ui, s| super::segmented(ui, &p, get(s), choices))
    }

    /// A colour.
    pub fn color(&mut self, label: &str, hint: &str, f: &F<crate::settings::Rgba>) -> bool {
        let (p, get) = (self.p, f.get_mut);
        self.row(label, hint, f, |ui, s| super::swatch(ui, &p, ui.scope_id().with("swatch"), get(s)))
    }

    /// A row that opens another page (`button` on its link).
    pub fn link(&mut self, label: &str, hint: &str, button: &str, page: Page, tab: usize) {
        let p = self.p;
        self.row_with(label, hint, &Nothing, |ui, _, st, _| {
            if crate::ui::widgets::action_button(ui, &p, "arrow-right", button, crate::ui::widgets::ActionKind::Flat, vec2(0.0, 26.0))
                .clicked()
            {
                st.go_to = Some((page, tab, None));
            }
            false
        });
    }

    fn row_id(&self, label: &str) -> egui::Id {
        let parent = self.parent.as_ref().map_or("", |p| p.label.as_str());
        egui::Id::unique(("preferences-row", self.page, self.group.as_str(), parent, label))
    }

    /// The search finds a row by its name, its help and its choices; not
    /// by its page or group, which would list a whole page for one word.
    fn matches(&self, label: &str, hint: &str, keywords: &str) -> bool {
        !self.words.is_empty() && found(&format!("{label} {hint} {keywords}"), &self.words)
    }

    fn columns(&self) -> usize {
        if self.grid.width >= TWO_COLUMNS { 2 } else { 1 }
    }

    /// The rect of the next cell: one column, or the whole width.
    fn cell(&mut self, wide: bool) -> Rect {
        if (wide || self.columns() == 1) && self.grid.open && self.grid.col > 0 {
            self.end_line();
        }
        if !self.grid.open {
            let room = self.ui.available_rect_before_wrap();
            self.grid.left = room.left();
            self.grid.width = room.width();
            self.grid.top = room.top();
            self.grid.height = 0.0;
            self.grid.col = 0;
            self.grid.open = true;
        }
        let left = self.grid.left + self.grid.indent;
        let width = self.grid.width - self.grid.indent;
        let (x, w) = if wide || self.columns() == 1 {
            (left, width)
        } else {
            let w = (width - GAP) / 2.0;
            (left + self.grid.col as f32 * (w + GAP), w)
        };
        Rect::from_min_size(pos2(x, self.grid.top), vec2(w, f32::INFINITY))
    }

    fn placed(&mut self, height: f32, wide: bool) {
        self.grid.height = self.grid.height.max(height);
        if wide || self.columns() == 1 || self.grid.col == 1 {
            self.end_line();
        } else {
            self.grid.col = 1;
        }
    }

    /// Closes the line under way: the page's cursor goes below it.
    fn end_line(&mut self) {
        if !self.grid.open {
            return;
        }
        let rect = Rect::from_min_size(pos2(self.grid.left, self.grid.top), vec2(self.grid.width, self.grid.height));
        self.ui.advance_cursor_after_rect(rect);
        self.grid.open = false;
        self.grid.col = 0;
        self.grid.height = 0.0;
    }

    /// Paints the background of an open cog's block, now that its end is known.
    fn end_block(&mut self) {
        let Some((top, shape)) = self.block.take() else { return };
        self.end_line();
        let room = self.ui.available_rect_before_wrap();
        let rect = Rect::from_min_max(pos2(room.left(), top), pos2(room.right(), self.ui.cursor().top()));
        let radius = self.ui.visuals().window_corner_radius;
        self.ui.painter().set(shape, Shape::rect_filled(rect, radius, self.p.field));
    }

    fn group_title(&mut self, title: &str) {
        let p = self.p;
        let font = egui::FontId::new(11.0, crate::ui::fonts::semibold(self.ui.ctx()));
        let galley = self.ui.painter().layout_no_wrap(title.to_uppercase(), font, p.muted);
        let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), 26.0), Sense::hover());
        let text_left = rect.left() + 4.0;
        crate::ui::widgets::paint_row_text(self.ui.painter(), galley.clone(), text_left, rect.center().y, p.muted);
        let line_left = text_left + galley.size().x + 10.0;
        if line_left < rect.right() {
            self.ui
                .painter()
                .hline(line_left..=rect.right(), rect.center().y, crate::ui::widgets::divider_stroke(&p));
        }
        self.ui.add_space(2.0);
    }

    /// The title of a group in the search and the modified view: where its
    /// rows live, and the way there.
    fn crumb(&mut self) {
        let p = self.p;
        let page = self.page;
        let tab = self.tab;
        let tabs = page.tabs();
        let mut place = page.name().to_owned();
        if tabs.len() > 1
            && let Some(name) = tabs.get(tab)
        {
            place.push_str(" › ");
            place.push_str(name);
        }
        let group = self.group.clone();
        self.ui.add_space(6.0);
        let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), 26.0), Sense::hover());
        let ctx = self.ui.ctx().clone();
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &place,
            0.0,
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(&ctx), 12.5, p.ink),
        );
        if !group.is_empty() && group != place && !tabs.contains(&group.as_str()) {
            job.append(
                &format!("  ·  {group}"),
                0.0,
                egui::TextFormat::simple(egui::FontId::proportional(12.0), p.muted),
            );
        }
        let galley = self.ui.painter().layout_job(job);
        crate::ui::widgets::paint_row_text(self.ui.painter(), galley.clone(), rect.left() + 4.0, rect.center().y, p.ink);
        // "Open the page", at the right end of the title
        let open = aurora_i18n::tr!("ui-options-open-page");
        let label = self
            .ui
            .painter()
            .layout_no_wrap(open.to_owned(), egui::FontId::proportional(12.0), p.violet_light);
        let w = label.size().x + 26.0;
        let link = Rect::from_min_size(pos2(rect.right() - w, rect.center().y - 11.0), vec2(w, 22.0));
        let line_left = rect.left() + 4.0 + galley.size().x + 10.0;
        if line_left < link.left() - 8.0 {
            self.ui.painter().hline(
                line_left..=link.left() - 8.0,
                rect.center().y,
                crate::ui::widgets::divider_stroke(&p),
            );
        }
        let resp = self.ui.interact(
            link,
            egui::Id::unique(("preferences-crumb", page, tab, group.as_str())),
            Sense::click(),
        );
        if resp.hovered() {
            self.ui.painter().rect_filled(
                link,
                self.ui.visuals().widgets.inactive.corner_radius,
                p.violet.gamma_multiply(0.14),
            );
        }
        let ink = if resp.hovered() { p.violet_pale } else { p.violet_light };
        crate::ui::widgets::paint_row_text(self.ui.painter(), label, link.left() + 6.0, link.center().y, ink);
        crate::ui::widgets::paint_glyph(
            self.ui.painter(),
            "arrow-right",
            pos2(link.right() - 11.0, link.center().y),
            13.0,
            ink,
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.st.go_to = Some((page, tab, None));
        }
        self.ui.add_space(2.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &mut self,
        id: egui::Id,
        label: &str,
        hint: &str,
        t: &dyn Track,
        modified: bool,
        next: &Next,
        body: impl FnOnce(&mut egui::Ui, &mut Settings, &mut OptionsUi, &mut OptionsResult) -> bool,
    ) -> bool {
        let p = self.p;
        let search = matches!(self.mode, Mode::Search | Mode::Modified);
        let cog = if search { None } else { self.st.subs.get(&id).copied() };
        let open = !search && (next.unfolded || (cog.is_some() && self.st.open_subs.contains(&id)));
        let wide = next.wide || open;
        if open {
            self.end_line();
            let top = self.ui.cursor().top();
            let shape = self.ui.painter().add(Shape::Noop);
            self.block = Some((top, shape));
        }
        let cell = self.cell(wide);
        let parent_off = self.greyed || (!next.live && self.parent.as_ref().is_some_and(|p| p.off)) || self.rlva_locked(t);
        let size_key = id.with("control");
        let control = self
            .st
            .control_sizes
            .get(&size_key)
            .copied()
            .unwrap_or(vec2((cell.width() * 0.45).min(240.0), 24.0));
        let left = cell.left() + GUTTER;
        let right = cell.right() - 10.0;
        let cog_w = if cog.is_some() { COG_W + 8.0 } else { 0.0 };
        let go_w = if search { 26.0 } else { 0.0 };
        let mark_w = if next.experimental { 19.0 } else { 0.0 };
        // Keep a readable label when a column cannot fit the stable value
        // slot beside it (including narrow windows and larger fonts).
        let below = next.below || control.x + cog_w + 14.0 + mark_w + go_w + 90.0 > right - left;
        let label_w = if below {
            right - left - mark_w - go_w
        } else {
            (right - left - control.x - cog_w - 14.0 - mark_w - go_w).max(90.0)
        };

        // the label, its search prefix (the parent of a row behind a cog)
        // and the part the search found
        let ctx = self.ui.ctx().clone();
        let mut job = egui::text::LayoutJob::default();
        let text = |color| egui::TextFormat::simple(egui::FontId::proportional(13.0), color);
        if search && let Some(parent) = &self.parent {
            job.append(&format!("{} › ", parent.label), 0.0, text(p.muted));
        }
        match search.then(|| found_in(label, &self.words)).flatten() {
            Some((a, b)) => {
                job.append(&label[..a], 0.0, text(p.ink));
                let mut lit = text(p.ink);
                lit.background = p.teal.gamma_multiply(0.3);
                job.append(&label[a..b], 0.0, lit);
                job.append(&label[b..], 0.0, text(p.ink));
            }
            None => job.append(label, 0.0, text(p.ink)),
        }
        job.wrap.max_width = label_w.max(1.0);
        let galley = ctx.fonts_mut(|f| f.layout_job(job));
        let section_body = next.section_body && self.mode == Mode::Show;
        let text_h = if section_body { 0.0 } else { galley.size().y };
        let label_gap = if section_body { 0.0 } else { 6.0 };
        let height = if below {
            text_h + label_gap + control.y + 16.0
        } else {
            text_h.max(control.y).max(20.0) + 12.0
        }
        .max(ROW_MIN_H);
        let row = Rect::from_min_size(cell.min, vec2(cell.width(), height));
        let background = self.ui.painter().add(Shape::Noop);

        // the control, at the right end of the row (or under the label)
        let control_rect = if below {
            Rect::from_min_size(pos2(left, row.top() + 8.0 + text_h + label_gap), vec2(right - left, control.y))
        } else {
            Rect::from_min_size(pos2(right - control.x, row.center().y - control.y / 2.0), control)
        };
        let room = if below {
            control_rect
        } else {
            Rect::from_min_max(control_rect.min, pos2(right, control_rect.max.y))
        };
        let mut child = self.ui.new_child(
            egui::UiBuilder::new()
                .id_salt(id)
                .max_rect(room)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing = vec2(8.0, 6.0);
        child.spacing_mut().slider_width = super::SLIDER_W;
        child.spacing_mut().interact_size.x = self.number_width;
        if parent_off {
            child.disable();
        }
        let changed = body(&mut child, self.s, self.st, self.r);
        let used = child.min_rect().size();
        let measured = vec2(used.x.ceil(), used.y.max(20.0).ceil());
        if (measured - control).abs().max_elem() > 0.5 {
            self.st.control_sizes.insert(size_key, measured);
            ctx.request_discard("preferences row measured");
        }

        // the label
        let label_pos = if below {
            pos2(left, row.top() + 8.0)
        } else {
            pos2(left, row.center().y - text_h / 2.0)
        };
        let label_rect = Rect::from_min_size(label_pos, if section_body { vec2(control.x, 20.0) } else { galley.size() });
        let ink = if parent_off { p.muted } else { p.ink };
        if !section_body {
            self.ui.painter().galley(label_pos, galley.clone(), ink);
        }
        if !hint.is_empty() {
            self.ui.interact(label_rect, id.with("label"), Sense::hover()).on_hover_text(hint);
        }
        let mut after = label_rect.right() + 6.0;
        if next.experimental {
            let mark = Rect::from_center_size(pos2(after + 6.5, label_rect.center().y), Vec2::splat(13.0));
            crate::ui::widgets::experimental_mark(self.ui, &p, mark, id.with("experimental"));
            after += 19.0;
        }
        if search {
            let go = Rect::from_center_size(pos2(after + 10.0, label_rect.center().y), Vec2::splat(22.0));
            let resp = self
                .ui
                .interact(go, id.with("go"), Sense::click())
                .on_hover_text(aurora_i18n::tr!("ui-options-go-to-setting"));
            if resp.hovered() {
                self.ui
                    .painter()
                    .rect_filled(go, self.ui.visuals().widgets.inactive.corner_radius, p.raised);
            }
            let ink = if resp.hovered() { p.ink } else { p.muted };
            crate::ui::widgets::paint_glyph(self.ui.painter(), "arrow-right", go.center(), 13.0, ink);
            if resp.clicked() {
                self.st.go_to = Some((self.page, self.tab, Some(id)));
                self.st.open_parent = self.parent.as_ref().map(|p| p.id);
            }
        }

        // the cog of the rows behind this one, with their number
        if let Some((count, off)) = cog {
            let rect = Rect::from_min_size(pos2(control_rect.left() - 8.0 - COG_W, row.center().y - 12.0), vec2(COG_W, 24.0));
            let resp = self.ui.interact(rect, id.with("cog"), Sense::click());
            let radius = self.ui.visuals().widgets.inactive.corner_radius;
            if open {
                self.ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.22));
            } else if resp.hovered() {
                self.ui.painter().rect_filled(rect, radius, p.raised);
            }
            let alpha = if off && !open { 0.45 } else { 1.0 };
            let ink = if open {
                p.violet_pale
            } else if resp.hovered() {
                p.ink
            } else {
                p.muted
            };
            crate::ui::widgets::paint_glyph(
                self.ui.painter(),
                "settings-filled",
                pos2(rect.left() + 12.0, rect.center().y),
                13.0,
                ink.gamma_multiply(alpha),
            );
            let n = self.ui.painter().layout_no_wrap(
                count.to_string(),
                egui::FontId::new(11.0, crate::ui::fonts::semibold(&ctx)),
                ink.gamma_multiply(alpha),
            );
            crate::ui::widgets::paint_row_text(self.ui.painter(), n, rect.left() + 22.0, rect.center().y, ink);
            let tip = if open {
                aurora_i18n::tr!("ui-options-hide-related", count = count)
            } else {
                aurora_i18n::tr!("ui-options-show-related", count = count)
            };
            if resp.on_hover_text(tip).clicked() {
                if open {
                    self.st.open_subs.remove(&id);
                } else {
                    self.st.open_subs.insert(id);
                }
            }
        }

        // the modified dot: a click puts the row back to its default
        let mut reset = false;
        if modified && !self.rlva_locked(t) {
            let dot = Rect::from_center_size(pos2(cell.left() + GUTTER / 2.0 + 1.0, label_rect.center().y), Vec2::splat(16.0));
            let resp = self
                .ui
                .interact(dot, id.with("reset"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(aurora_i18n::tr!("ui-options-modified-reset"));
            if resp.hovered() {
                crate::ui::widgets::paint_glyph(self.ui.painter(), "restore", dot.center(), 13.0, p.violet_light);
            } else {
                self.ui.painter().circle_filled(dot.center(), 3.0, p.violet_light);
            }
            reset = resp.clicked();
        }
        if reset {
            t.reset(self.s, self.d);
            self.st.after_reset(self.page, self.s, self.r);
        }

        // the row's face: hovered, or lit a moment after "Go to"
        let hovered = self.ui.rect_contains_pointer(row) && self.ui.is_enabled();
        let flash = self.st.flash.filter(|(f, _)| *f == id).map(|(_, at)| at);
        let now = ctx.input(|i| i.time);
        let radius = self.ui.visuals().widgets.inactive.corner_radius;
        if let Some(at) = flash {
            let age = (now - at) as f32;
            if age < 1.8 {
                // asked again for a quarter of a second: a window opened by
                // "Preferences…" lays out its first frames without keeping
                // a scroll, which left a row far down the page out of sight
                if self.st.flash_scroll {
                    self.ui.scroll_to_rect(row, Some(egui::Align::Center));
                    self.st.flash_scroll = age < 0.25;
                }
                let strength = (1.0 - age / 1.8).clamp(0.0, 1.0) * 0.42;
                self.ui
                    .painter()
                    .set(background, Shape::rect_filled(row, radius, p.violet.gamma_multiply(strength)));
                ctx.request_repaint();
            } else {
                self.st.flash = None;
            }
        } else if hovered {
            self.ui
                .painter()
                .set(background, Shape::rect_filled(row, radius, p.raised.gamma_multiply(0.35)));
        }
        self.placed(height, wide);
        changed || reset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_ignores_case_and_accents() {
        assert_eq!(fold("Qualité des Ombres"), "qualite des ombres");
        assert_eq!(fold("Œuvre"), "oeuvre");
        assert_eq!(words("  Ombré  ÉCUME "), vec!["ombre".to_owned(), "ecume".to_owned()]);
        assert!(found("Qualité des ombres", &words("OMBRE")));
    }

    #[test]
    fn the_search_finds_the_start_of_words_only() {
        assert!(!found("Nombre de sondes", &words("ombre")));
        assert!(found("Nombre de sondes", &words("nomb sond")));
        assert!(!found("Nombre de sondes", &words("nomb ombre")));
        // a shortcut by its keys
        assert!(found("Carte du monde Ctrl+M", &words("ctrl+m")));
    }

    #[test]
    fn the_part_found_is_lit_in_the_label_as_written() {
        let w = words("ecume");
        let label = "Écume sur les rivages";
        let (a, b) = found_in(label, &w).expect("found");
        assert_eq!(&label[a..b], "Écume");
        assert_eq!(found_in("Bulles", &w), None);
        assert_eq!(found_in("Nombre", &words("ombre")), None);
    }
}
