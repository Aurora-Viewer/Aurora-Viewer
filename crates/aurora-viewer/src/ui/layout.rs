//! UI layout kept between sessions (the Legacy Viewer saves floater rects):
//! egui's memory (window positions and sizes, collapsed floaters) is written
//! to `ui_layout.ron` shortly after each change (`Autosave`) and on exit, and
//! restored at startup. Which panels are open lives in the settings.
//!
//! Each window retains one of eight invisible perimeter anchors and its
//! signed offset in the room where windows may move. Existing default rects
//! initialize these points and offsets directly; a window may supply an
//! explicit default point. First opening and completed moves otherwise choose
//! the closest corresponding corners or side midpoints, retained on resize.
//! `Slot` shares this placement with floaters, toolstrips and the mini-map.
//!
//! The slots also make the windows of the frame known to the gestures
//! (`gesture`): a window moved with Alt or resized snaps to the others and to the
//! work area (`snap`), and windows snapped side by side are glued: they
//! move as one block, saved with the layout too.

mod anchor;
mod gesture;

use super::snap;
use crate::theme::Palette;
use anchor::Anchor;
use egui::{Align2, Id, Pos2, Rect, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

fn path() -> PathBuf {
    crate::settings::config_dir().join("ui_layout.ron")
}

/// Restore the saved layout. The style and options set by the skin are
/// kept (only positions, sizes and per-window state come from the file).
///
/// Call it before the theme and `fonts::apply`: egui keeps the transient
/// font scale and the font definitions that wait
/// for the next pass in the memory this replaces, so fonts installed before
/// it are dropped and the interface starts with egui's own.
pub fn load(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read(path()) else {
        return;
    };
    if let Err(e) = std::str::from_utf8(&bytes).map_err(|e| e.to_string()).and_then(|s| restore(ctx, s)) {
        log::warn!("ui layout ignored: {e}");
    }
}

fn restore(ctx: &egui::Context, saved: &str) -> Result<(), String> {
    let mem = ron::from_str::<egui::Memory>(saved).map_err(|e| e.to_string())?;
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = mem);
    ctx.options_mut(|o| *o = options);
    Ok(())
}

/// Save the layout now (on exit, after a restored backup).
pub fn save(ctx: &egui::Context) {
    let (generation, mem) = snapshot(ctx);
    write(generation, &mem);
}

/// The layout as `ui_layout.ron` holds it (and a settings backup).
pub fn text(ctx: &egui::Context) -> Option<String> {
    let (_, mem) = snapshot(ctx);
    ron::to_string(&mem).inspect_err(|e| log::warn!("ui layout not saved: {e}")).ok()
}

/// Order of the saves: each snapshot takes the next number, and the file
/// only takes a snapshot newer than the last one written (or than a reset),
/// so a background save finishing after the exit's never brings an older
/// layout back.
static ISSUED: AtomicU64 = AtomicU64::new(0);
static WRITTEN: Mutex<Written> = Mutex::new(Written { generation: 0, text: None });

struct Written {
    generation: u64,
    /// What the file holds, to skip writing the same layout again.
    text: Option<String>,
}

/// The current layout, with the windows' anchors up to date. Cheap enough
/// for the frame (a clone); serializing it is left to `write`.
fn snapshot(ctx: &egui::Context) -> (u64, egui::Memory) {
    let room = move_area(ctx);
    windows(ctx, |w| w.refresh_anchors(room));
    (next_generation(), ctx.memory(|m| m.clone()))
}

fn next_generation() -> u64 {
    ISSUED.fetch_add(1, Ordering::Relaxed) + 1
}

fn write(generation: u64, mem: &egui::Memory) {
    write_to(&WRITTEN, &path(), generation, mem);
}

fn write_to(state: &Mutex<Written>, p: &Path, generation: u64, mem: &egui::Memory) {
    let text = match ron::to_string(mem) {
        Ok(text) => text,
        Err(e) => {
            log::warn!("ui layout not saved: {e}");
            return;
        }
    };
    let mut written = state.lock().unwrap_or_else(PoisonError::into_inner);
    if generation <= written.generation {
        return;
    }
    written.generation = generation;
    if written.text.as_deref() == Some(text.as_str()) {
        return;
    }
    if let Some(dir) = p.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("ui layout not saved: {e}");
        return;
    }
    // write then rename: never leave a half-written file
    match crate::atomic_file::write(p, text.as_bytes()) {
        Ok(()) => written.text = Some(text),
        Err(e) => log::warn!("ui layout not saved: {e}"),
    }
}

/// How long the layout must stay still before it is saved: a window moved
/// several times in a row is written once.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

/// Saves the layout shortly after the user changes it (a window moved,
/// resized or collapsed, a column widened, the viewer window resized), on a
/// background thread: a crash or a forced close no longer loses the
/// arrangement of the session (#1319). The Legacy Viewer keeps the floater
/// rects in its settings and writes them at exit only
/// (LLFloater::storeRectControl, indra/llui/llfloater.cpp;
/// LLAppViewer::cleanup, indra/newview/llappviewer.cpp): a clean exit gives
/// the same file, an abrupt one no longer loses it.
#[derive(Default)]
pub struct Autosave {
    due: Option<Instant>,
    room: Option<Rect>,
}

impl Autosave {
    /// Call it after each egui pass.
    pub fn after_pass(&mut self, ctx: &egui::Context, now: Instant) {
        let room = move_area(ctx);
        let released = ctx.input(|i| i.pointer.any_released()) && (ctx.is_pointer_over_egui() || ctx.egui_is_using_pointer());
        let resized = self.room.is_some_and(|r| r != room);
        self.room = Some(room);
        if self.mark(released || resized, now) {
            let (generation, mem) = snapshot(ctx);
            let spawned = std::thread::Builder::new()
                .name("aurora-layout-save".into())
                .spawn(move || write(generation, &mem));
            if let Err(e) = spawned {
                log::warn!("ui layout not saved: {e}");
            }
        }
    }

    /// Whether the layout is due for a save this frame: `changed` pushes the
    /// save back by `AUTOSAVE_DELAY`.
    fn mark(&mut self, changed: bool, now: Instant) -> bool {
        if changed {
            self.due = Some(now + AUTOSAVE_DELAY);
        }
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            return true;
        }
        false
    }
}

/// Takes a layout from a settings backup, at once and for the next
/// launches.
pub fn apply(ctx: &egui::Context, saved: &str) -> Result<(), String> {
    restore(ctx, saved)?;
    save(ctx);
    Ok(())
}

/// Forget the saved layout ("Reset everything").
pub fn forget(ctx: &egui::Context) {
    discard(&WRITTEN, &path(), next_generation());
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = egui::Memory::default());
    ctx.options_mut(|o| *o = options);
}

/// Remove the file; a background save still on its way, older than
/// `generation`, does not write it back.
fn discard(state: &Mutex<Written>, p: &Path, generation: u64) {
    let mut written = state.lock().unwrap_or_else(PoisonError::into_inner);
    written.generation = written.generation.max(generation);
    written.text = None;
    let _ = std::fs::remove_file(p);
}

/// Room left between a window and the bars or the edges of the screen, and
/// between two windows that stand side by side, for the windows of a first
/// launch as for the ones the user snaps (`snap`): none, their borders
/// touch, as Firestorm's snapped floaters do (findSnapRect is called
/// without padding).
pub(crate) const GAP: f32 = 0.0;

/// The part of `screen` the windows are laid out in: between the navigation
/// bar and the bottom bar of the default skin. For what must be known before
/// the bars are drawn; a frame then reports the real one (`set_work_area`).
pub(crate) fn default_work_area(screen: Rect) -> Rect {
    let bars = super::skin::Layout::default();
    Rect::from_min_max(
        Pos2::new(screen.left(), screen.top() + bars.nav_bar_height),
        Pos2::new(screen.right(), screen.bottom() - bars.bottom_bar_height),
    )
}

fn work_area_id() -> Id {
    Id::unique("layout_work_area")
}

/// The room left by the bars this frame: call it once they are drawn,
/// before the windows are shown.
pub(crate) fn set_work_area(ctx: &egui::Context, area: Rect) {
    ctx.data_mut(|d| d.insert_temp(work_area_id(), area));
}

/// The room the bars leave this frame, from `left` (what the panels of
/// Firestorm's bars leave) and `bottom_top` (the top of what the bottom
/// bar keeps): with the islands, below the top islands and above the dock,
/// 8 px from the window's sides (`bars::islands::work_area`).
pub(crate) fn bars_room(left: Rect, bottom_top: f32, classic_bars: bool) -> Rect {
    if classic_bars {
        left
    } else {
        super::bars::islands::work_area(left, bottom_top)
    }
}

/// Where the windows are laid out: the screen without the bars.
pub(crate) fn work_area(ctx: &egui::Context) -> Rect {
    ctx.data(|d| d.get_temp::<Rect>(work_area_id()))
        .unwrap_or_else(|| default_work_area(ctx.content_rect()))
}

fn move_area_id() -> Id {
    Id::unique("layout_move_area")
}

/// Where the windows can be moved this frame: with the islands the whole
/// screen, since the islands leave room between them, above and below;
/// with the Legacy Viewer's full-width bars, the room between them. Call
/// it with `set_work_area`.
pub(crate) fn set_move_area(ctx: &egui::Context, area: Rect) {
    ctx.data_mut(|d| d.insert_temp(move_area_id(), area));
}

/// Where the windows can be moved (`set_move_area`); the work area when
/// nothing set it.
pub(crate) fn move_area(ctx: &egui::Context) -> Rect {
    ctx.data(|d| d.get_temp::<Rect>(move_area_id())).unwrap_or_else(|| work_area(ctx))
}

/// Height of a floater's title bar, and the width of it that stays in the
/// room the windows move in at least: enough to grab it and bring the
/// window back.
pub(crate) const TITLE_BAR_H: f32 = 38.0;
pub(crate) const TITLE_GRIP: f32 = 96.0;

/// Which edges of the room stop the windows (`Settings::windows_stop_*`,
/// all by default): a window stays whole on the screen on that side; off,
/// it may cross that edge, its title bar kept within reach. The top edge
/// always stops them: a title bar above it could not be grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edges {
    pub left: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Default for Edges {
    fn default() -> Self {
        Edges {
            left: true,
            right: true,
            bottom: true,
        }
    }
}

impl Edges {
    pub const OPEN: Self = Self {
        left: false,
        right: false,
        bottom: false,
    };
}

fn edges_id() -> Id {
    Id::unique("layout_window_edges")
}

/// The edges that stop the windows this frame (the user's settings): call
/// it with `set_move_area`.
pub(crate) fn set_window_edges(ctx: &egui::Context, edges: Edges) {
    ctx.data_mut(|d| d.insert_temp(edges_id(), edges));
}

/// The edges that stop the windows (`set_window_edges`); all of them when
/// nothing set it.
pub(crate) fn window_edges(ctx: &egui::Context) -> Edges {
    ctx.data(|d| d.get_temp::<Edges>(edges_id())).unwrap_or_default()
}

/// Per-window exception to the edge stops, retaining the shared title grip.
pub(crate) fn set_partial_outside(ctx: &egui::Context, id: Id, allowed: bool) {
    ctx.data_mut(|d| d.insert_temp(id.with("partial_outside"), allowed));
}

fn partial_outside(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id.with("partial_outside"))).unwrap_or(false)
}

pub(crate) fn edges_for(ctx: &egui::Context, id: Id) -> Edges {
    if partial_outside(ctx, id) { Edges::OPEN } else { window_edges(ctx) }
}

/// The rect a window of `size` is kept in (`egui::Window::constrain_to`):
/// whole in `area` on the sides whose edge stops it (`edges`); past the
/// left or right edge otherwise but `TITLE_GRIP` of its title bar, past the
/// bottom one but its whole title bar. Never above the top.
pub(crate) fn reach(area: Rect, size: Vec2, edges: Edges) -> Rect {
    let grip = TITLE_GRIP.min(size.x);
    let over_x = (size.x - grip).max(0.0);
    let over_y = (size.y - TITLE_BAR_H).max(0.0);
    Rect::from_min_max(
        Pos2::new(area.left() - if edges.left { 0.0 } else { over_x }, area.top()),
        Pos2::new(
            area.right() + if edges.right { 0.0 } else { over_x },
            area.bottom() + if edges.bottom { 0.0 } else { over_y },
        ),
    )
}

/// `pos` of a window of `size`, moved within `reach`; a window larger than
/// the room keeps its top left corner in it.
pub(crate) fn keep_title_in(area: Rect, pos: Pos2, size: Vec2, edges: Edges) -> Pos2 {
    let limits = reach(area, size, edges);
    let right = (limits.right() - size.x).max(limits.left());
    let bottom = (limits.bottom() - size.y).max(limits.top());
    Pos2::new(pos.x.clamp(limits.left(), right), pos.y.clamp(limits.top(), bottom))
}

fn islands_id() -> Id {
    Id::unique("layout_bar_islands")
}

/// An island of the bars was drawn this frame at `rect` (`bars::islands`):
/// the windows keep their title clear of them (`clear_of_islands`).
pub(crate) fn island_drawn(ctx: &egui::Context, rect: Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let islands = d.get_temp_mut_or_default::<(u64, Vec<Rect>)>(islands_id());
        if islands.0 != frame {
            *islands = (frame, Vec::new());
        }
        islands.1.push(rect);
    });
}

/// The islands drawn this frame.
pub(crate) fn islands(ctx: &egui::Context) -> Vec<Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, Vec<Rect>)>(islands_id()))
        .filter(|(drawn, _)| *drawn == frame)
        .map(|(_, rects)| rects)
        .unwrap_or_default()
}

/// How much of `title` (a title bar) shows in `area` beside the islands
/// that cross its height.
fn uncovered(title: Rect, area: Rect, islands: &[Rect]) -> f32 {
    let (left, right) = (title.left().max(area.left()), title.right().min(area.right()));
    if right <= left {
        return 0.0;
    }
    let mut covered: Vec<(f32, f32)> = islands
        .iter()
        .filter(|i| i.top() < title.bottom() && i.bottom() > title.top())
        .map(|i| (i.left().max(left), i.right().min(right)))
        .filter(|(a, b)| b > a)
        .collect();
    covered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut hidden = 0.0;
    let mut end = left;
    for (a, b) in covered {
        if b > end {
            hidden += b - a.max(end);
            end = b;
        }
    }
    (right - left) - hidden
}

/// Room kept between a window brought out from under the islands and them.
const ISLAND_CLEARANCE: f32 = 8.0;

/// `pos` of a window of `size`, moved off the islands when its title bar is
/// hidden by them all but `TITLE_GRIP` (the islands and the windows share a
/// layer: the last one clicked comes on top, and a title left under an
/// island could not be grabbed again). It goes below the islands of the
/// upper half of `area`, above those of the lower half, and stays within
/// reach (`keep_title_in`).
pub(crate) fn clear_of_islands(area: Rect, pos: Pos2, size: Vec2, edges: Edges, islands: &[Rect]) -> Pos2 {
    let width = size.x;
    let title = Rect::from_min_size(pos, Vec2::new(width, TITLE_BAR_H));
    if uncovered(title, area, islands) >= TITLE_GRIP.min(width) {
        return pos;
    }
    let crossing: Vec<&Rect> = islands
        .iter()
        .filter(|i| i.top() < title.bottom() && i.bottom() > title.top() && i.left() < title.right() && i.right() > title.left())
        .collect();
    let upper = crossing.iter().filter(|i| i.center().y < area.center().y);
    let lower = crossing.iter().filter(|i| i.center().y >= area.center().y);
    let y = if let Some(bottom) = upper.map(|i| i.bottom()).reduce(f32::max) {
        bottom + ISLAND_CLEARANCE
    } else if let Some(top) = lower.map(|i| i.top()).reduce(f32::min) {
        top - ISLAND_CLEARANCE - TITLE_BAR_H
    } else {
        pos.y
    };
    keep_title_in(area, Pos2::new(pos.x, y), size, edges)
}

/// A screen smaller than this is not one to lay windows out in: the content
/// rect of a minimized viewer window, or of the first frames before the
/// surface has its size. Windows neither follow it nor record it.
const MIN_SCREEN: Vec2 = Vec2::new(320.0, 240.0);

fn usable(screen: Rect) -> bool {
    screen.is_finite() && screen.width() >= MIN_SCREEN.x && screen.height() >= MIN_SCREEN.y
}

/// Where a window stood the last time it was shown, and in which screen.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Placed {
    rect: Rect,
    screen: Rect,
    /// Absent in older layouts, which recorded only the full screen.
    #[serde(default)]
    room: Option<Rect>,
    #[serde(default)]
    anchor: Option<Anchor>,
}

/// A window shown this frame or the last one.
#[derive(Clone, Copy, Debug)]
struct Shown {
    id: Id,
    rect: Rect,
    /// The frame (`Live::frame`) it was last shown in.
    frame: u64,
    minimized: bool,
    partial_outside: bool,
}

/// What is known of the windows for the session only.
#[derive(Clone, Debug, Default)]
struct Live {
    /// Frames counted by `frame` (the gestures); stays 0 without it.
    frame: u64,
    shown: BTreeMap<u64, Shown>,
    /// The screen the targets of `follow` were computed for, and where each
    /// window recorded in another screen goes in it.
    followed: Option<(Rect, Rect)>,
    follow: BTreeMap<u64, Pos2>,
    /// Positions the gestures give this frame: the windows glued to the one
    /// dragged, a snap applied.
    places: BTreeMap<u64, Pos2>,
    /// Windows egui measures this frame, and where each belongs: egui keeps
    /// a window it has no size for inside the screen as if it had its
    /// default size, which pushes away from the bottom and right edges one
    /// that was left smaller. The next frame puts it back.
    settle: BTreeMap<u64, Pos2>,
    /// Where the dragged window of a group can go (`snap::block_limits`).
    limits: Option<(u64, Rect)>,
    /// The setting (`Settings::snap_windows`), as of this frame.
    snap: bool,
    moved: Option<gesture::Move>,
    resized: Option<gesture::Resize>,
    /// Completed gestures or explicit placements, recorded after every
    /// member has drawn its final rect, including any applied snap.
    reanchor: BTreeSet<u64>,
    /// Explicit placements may be requested before layout::frame runs;
    /// wait until Slot::shown has recorded their actual result.
    reanchor_after_show: BTreeSet<u64>,
}

/// The windows the user places, by `Id::value`; persisted with egui's
/// memory, so written to `ui_layout.ron` with the rest of the layout.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Windows {
    #[serde(default)]
    placed: BTreeMap<u64, Placed>,
    /// Windows snapped side by side: they move together (`snap::group`).
    #[serde(default)]
    links: BTreeSet<snap::Link>,
    #[serde(skip)]
    live: Live,
}

impl Windows {
    fn anchor_group(&mut self, window: u64, room: Rect) {
        let members = snap::group(&self.links, window);
        let block = members
            .iter()
            .filter_map(|id| self.placed.get(id))
            .fold(Rect::NOTHING, |all, placed| all.union(placed.rect));
        if !block.is_finite() {
            return;
        }
        let point = Anchor::nearest(room, block).point;
        for id in members {
            if let Some(placed) = self.placed.get_mut(&id) {
                placed.anchor = Some(Anchor::new(point, room, placed.rect));
            }
        }
        self.live.followed = None;
    }

    fn refresh_anchors(&mut self, room: Rect) {
        for id in std::mem::take(&mut self.live.reanchor) {
            self.anchor_group(id, room);
        }
    }

    /// Reconstruct positions from saved anchors, including closed windows.
    /// Compute once before any window is recorded in the new room.
    fn follow(&mut self, window: u64, screen: Rect, room: Rect) -> Option<Pos2> {
        if self.live.followed == Some((screen, room)) {
            return self.live.follow.remove(&window);
        }
        // Migrate old layouts before any member moves to the current room.
        let missing: Vec<_> = self
            .placed
            .iter()
            .filter(|(_, p)| p.anchor.is_none() && usable(p.screen) && p.rect.is_finite())
            .map(|(id, p)| (*id, *p))
            .collect();
        for (id, placed) in missing {
            if self.placed[&id].anchor.is_none() {
                // Old layouts knew the screen but not the movement room.
                // Transfer the current bar insets to that screen so migration
                // does not add the bars' height to an existing window offset.
                let old_room = placed.room.unwrap_or_else(|| {
                    Rect::from_min_max(
                        placed.screen.min + (room.min - screen.min),
                        placed.screen.max + (room.max - screen.max),
                    )
                });
                self.anchor_group(id, old_room);
            }
        }
        self.live.followed = Some((screen, room));
        self.live.follow = self
            .placed
            .iter()
            .filter(|(_, placed)| placed.screen != screen || placed.room != Some(room))
            .filter_map(|(id, placed)| {
                let pos = placed.anchor?.position(room, placed.rect.size());
                pos.is_finite().then_some((*id, pos))
            })
            .collect();
        self.live.follow.remove(&window)
    }
}

fn windows<R>(ctx: &egui::Context, read: impl FnOnce(&mut Windows) -> R) -> R {
    ctx.data_mut(|d| read(d.get_persisted_mut_or_default::<Windows>(Id::unique("window_layout"))))
}

/// One window of the layout for one frame: ask for it before the window is
/// shown, give `pos` and `limits` (when there are) to
/// `egui::Window::current_pos` and `constrain_to`, and report the rect once
/// shown.
#[must_use = "report the window with Slot::shown"]
pub(crate) struct Slot {
    id: Id,
    /// egui knew the size of the window before this frame. It does not on
    /// the first frame of a window, shown or restored from the layout: that
    /// frame only measures it, and its rect is not one to record.
    sized: bool,
    preserve_anchor: bool,
    initial_anchor: Option<Align2>,
    /// The position the window takes this frame: it follows the window it
    /// is glued to, a snap is applied, or the screen changed since it was
    /// last shown and it follows its edges.
    pub pos: Option<Pos2>,
    /// Where the window can be dragged this frame instead of the whole
    /// screen: it carries a group that must stay inside with it.
    pub limits: Option<Rect>,
}

/// The slot of window `id` (the id given to `egui::Window::id`).
pub(crate) fn slot(ctx: &egui::Context, id: Id) -> Slot {
    let screen = ctx.content_rect();
    let room = move_area(ctx);
    let sized = egui::AreaState::load(ctx, id).is_some_and(|s| s.size.is_some());
    let key = id.value();
    let (pos, limits, preserve_anchor) = windows(ctx, |w| {
        let followed = usable(screen).then(|| w.follow(key, screen, room)).flatten();
        let preserve_anchor = followed.is_some() || !sized || w.live.settle.contains_key(&key);
        let limits = w.live.limits.filter(|(window, _)| *window == key).map(|(_, limits)| limits);
        let mut pos = w.live.places.remove(&key).or(followed);
        if sized {
            // measured last frame: back where it belongs, unless it is
            // given another place now
            let settled = w.live.settle.remove(&key);
            pos = pos.or(settled);
        } else if let Some(target) = pos.or_else(|| w.placed.get(&key).map(|placed| placed.rect.min)) {
            w.live.settle.insert(key, target);
        }
        (pos, limits, preserve_anchor)
    });
    Slot {
        id,
        sized,
        preserve_anchor,
        initial_anchor: None,
        pos,
        limits,
    }
}

impl Slot {
    /// An explicit default is applied only after measurement and before the
    /// first recorded placement. Saved layouts and requested moves take priority.
    pub(crate) fn default_anchor(mut self, ctx: &egui::Context, point: Align2, at: Pos2) -> Self {
        let key = self.id.value();
        let fresh = windows(ctx, |w| {
            !w.placed.contains_key(&key) && !w.live.reanchor_after_show.contains(&key) && !w.live.reanchor.contains(&key)
        });
        if fresh {
            self.initial_anchor = Some(point);
            if self.pos.is_none()
                && let Some(size) = egui::AreaState::load(ctx, self.id).and_then(|s| s.size)
            {
                self.pos = Some(point.anchor_size(at, size).min);
            }
        }
        self
    }

    /// The window was shown at `rect`; `minimized`: reduced to its title.
    pub(crate) fn shown(self, ctx: &egui::Context, rect: Rect, minimized: bool) {
        let screen = ctx.content_rect();
        let room = move_area(ctx);
        let partial_outside = partial_outside(ctx, self.id);
        windows(ctx, |w| {
            let key = self.id.value();
            let recorded = w.placed.get(&key).copied();
            let rect = match (self.sized, recorded) {
                // measured this frame: it keeps the size it was recorded
                // with, where it belongs (`Live::settle`)
                (false, Some(old)) => {
                    let min = w.live.settle.get(&key).copied().unwrap_or(rect.min);
                    Rect::from_min_size(min, old.rect.size())
                }
                _ => rect,
            };
            let anchor = recorded
                .and_then(|old| {
                    old.anchor.map(|anchor| {
                        // Content growth and collapse change the window's own
                        // anchor point. Keep its current position and chosen
                        // point; viewer resize constraints never rewrite offsets.
                        if !self.preserve_anchor && old.room == Some(room) && old.screen == screen && old.rect.size() != rect.size() {
                            Anchor::new(anchor.point, room, rect)
                        } else {
                            anchor
                        }
                    })
                })
                .or_else(|| {
                    (self.sized && usable(screen)).then(|| {
                        self.initial_anchor
                            .map_or_else(|| Anchor::nearest(room, rect), |point| Anchor::new(point, room, rect))
                    })
                });
            let new = Placed {
                rect,
                screen,
                room: Some(room),
                anchor,
            };
            if usable(screen) && (self.sized || recorded.is_some()) && recorded != Some(new) {
                w.placed.insert(key, new);
            }
            if usable(screen) && self.sized && w.live.reanchor_after_show.remove(&key) {
                w.live.reanchor.insert(key);
            }
            let frame = w.live.frame;
            w.live.shown.insert(
                key,
                Shown {
                    id: self.id,
                    rect,
                    frame,
                    minimized,
                    partial_outside,
                },
            );
        });
    }
}

/// An explicit placement follows the same rule as a completed mouse move.
pub(crate) fn reanchor(ctx: &egui::Context, id: Id) {
    windows(ctx, |w| {
        w.live.reanchor_after_show.insert(id.value());
    });
}

/// The gestures of the frame on the windows: call it once per frame, after
/// the bars (`set_work_area`) and before the windows are shown. A window
/// dragged by its title takes the windows glued to it along, is offered a
/// snap to the others and to the work area only while Alt is held (`snap`;
/// `snap_enabled` is the user's setting), shown by a guide and applied when
/// the mouse is released; Alt also takes it out of its current group.
pub(crate) fn frame(ctx: &egui::Context, p: &Palette, snap_enabled: bool) {
    let screen = ctx.content_rect();
    if !usable(screen) {
        return;
    }
    let pointer = gesture::Pointer {
        dragged: ctx.dragged_id(),
        stopped: ctx.drag_stopped_id(),
        total: ctx.input(|i| i.pointer.total_drag_delta()).unwrap_or_default(),
        alt: ctx.input(|i| i.modifiers.alt),
        time: ctx.input(|i| i.time),
    };
    let (area, room, edges, pixels_per_point) = (work_area(ctx), move_area(ctx), window_edges(ctx), ctx.pixels_per_point());
    let overlay = windows(ctx, |w| w.drive(area, room, edges, pixels_per_point, pointer, snap_enabled));
    if let Some(overlay) = overlay {
        snap::overlay::paint(ctx, p, &overlay);
    }
}

/// Window `id` is resized by its `edges` and the pointer has it at `rect`:
/// the rect to give it, its moving edges snapped when they come near
/// another window or the work area. Unlike a move, a resize snaps under the
/// pointer: the window is laid out again at each size, and its content may
/// refuse one (`min_size` grows then, and the snap is no longer offered).
pub(crate) fn resizing(ctx: &egui::Context, p: &Palette, id: Id, rect: Rect, edges: snap::Edges, min_size: Vec2) -> Rect {
    let (alt, time, released) = ctx.input(|i| (i.modifiers.alt, i.time, !i.pointer.primary_down()));
    let area = work_area(ctx);
    let request = gesture::ResizeRequest {
        window: id.value(),
        rect,
        edges,
        min_size,
        alt,
        time,
        released,
    };
    let (rect, overlay) = windows(ctx, |w| w.resize(request, area));
    if let Some(overlay) = overlay {
        snap::overlay::paint(ctx, p, &overlay);
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))
    }

    /// Frames of the windows of a session: a floater against the left edge,
    /// one in the bottom right corner, and a bare `egui::Window` in the top
    /// right one, like the mini-map.
    struct Session {
        ctx: egui::Context,
        frame: usize,
    }

    const LEFT: &str = "layout_left";
    const CORNER: &str = "layout_corner";
    const MAP: &str = "layout_map";

    impl Session {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                frame: 0,
            }
        }

        /// One frame in a `width` x `height` screen; the windows open at
        /// their place in a 1280 x 720 one.
        fn frame(&mut self, width: f32, height: f32) {
            self.frame += 1;
            let palette = crate::theme::Theme::default().palette();
            let input = egui::RawInput {
                screen_rect: Some(screen(width, height)),
                time: Some(self.frame as f64 / 60.0),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| {
                let ctx = ui.ctx();
                frame(ctx, &palette, false);
                let floater = |id, pos| {
                    crate::ui::widgets::Floater::new(id, "Window", pos, Vec2::new(300.0, 200.0))
                        .silent()
                        .show(ctx, &palette, &mut true, |ui| {
                            ui.label("content");
                        });
                };
                floater(LEFT, Pos2::new(8.0, 56.0));
                floater(CORNER, Pos2::new(972.0, 480.0));
                let slot = slot(ctx, Id::unique(MAP));
                let mut map = egui::Window::new("Map")
                    .id(Id::unique(MAP))
                    .title_bar(false)
                    .default_pos([1064.0, 56.0])
                    .default_size([200.0, 200.0]);
                if let Some(pos) = slot.pos {
                    map = map.current_pos(pos);
                }
                if let Some(shown) = map.show(ctx, |ui| ui.allocate_space(ui.available_size())) {
                    slot.shown(ctx, shown.response.rect, false);
                }
            });
            output.textures_delta.clear();
        }

        fn frames(&mut self, n: usize, width: f32, height: f32) {
            for _ in 0..n {
                self.frame(width, height);
            }
        }

        fn rect(&self, id: &str) -> Rect {
            self.ctx.memory(|m| m.area_rect(Id::unique(id))).expect("the window was shown")
        }

        /// (left gap, top gap, right gap, bottom gap) of a window.
        fn gaps(&self, id: &str, width: f32, height: f32) -> [f32; 4] {
            let r = self.rect(id);
            [r.left(), r.top(), width - r.right(), height - r.bottom()]
        }

        /// The session that follows: a new context restored from the layout
        /// this one would write on exit.
        fn restart(&self) -> Session {
            let saved = ron::to_string(&self.ctx.memory(|m| m.clone())).expect("serialize");
            let next = Session::new();
            restore(&next.ctx, &saved).expect("restore");
            next
        }
    }

    #[test]
    fn windows_follow_their_edges_when_the_viewer_window_is_resized() {
        let mut s = Session::new();
        s.frames(3, 1280.0, 720.0);
        let left = s.gaps(LEFT, 1280.0, 720.0);
        let corner = s.gaps(CORNER, 1280.0, 720.0);
        let map = s.gaps(MAP, 1280.0, 720.0);
        assert_eq!((left[0], left[1]), (8.0, 56.0));
        assert_eq!((map[1], map[2]), (56.0, 16.0), "{:?}", s.rect(MAP));
        for (width, height) in [(1600.0, 900.0), (1920.0, 1080.0), (1400.0, 760.0), (1280.0, 720.0)] {
            s.frames(2, width, height);
            let now = s.gaps(LEFT, width, height);
            assert_eq!((now[0], now[1]), (left[0], left[1]), "{width} x {height}");
            let now = s.gaps(CORNER, width, height);
            assert_eq!((now[2], now[3]), (corner[2], corner[3]), "{width} x {height}");
            let now = s.gaps(MAP, width, height);
            assert_eq!((now[1], now[2]), (map[1], map[2]), "{width} x {height}");
        }
    }

    #[test]
    fn first_opening_initializes_anchors_without_a_manual_move() {
        let mut s = Session::new();
        s.frames(3, 1280.0, 720.0);
        for (id, point) in [
            (LEFT, egui::Align2::LEFT_TOP),
            (CORNER, egui::Align2::RIGHT_BOTTOM),
            (MAP, egui::Align2::RIGHT_TOP),
        ] {
            let placed = windows(&s.ctx, |w| w.placed[&Id::unique(id).value()]);
            assert_eq!(placed.anchor.expect("initialized at measured opening").point, point);
            assert_eq!(placed.room, Some(move_area(&s.ctx)));
        }
        let anchors = windows(&s.ctx, |w| {
            w.placed.iter().map(|(id, p)| (*id, p.anchor)).collect::<BTreeMap<_, _>>()
        });
        for (width, height) in [(640.0, 480.0), (1920.0, 1080.0), (1280.0, 720.0)] {
            s.frames(3, width, height);
            assert_eq!(
                windows(&s.ctx, |w| w
                    .placed
                    .iter()
                    .map(|(id, p)| (*id, p.anchor))
                    .collect::<BTreeMap<_, _>>()),
                anchors
            );
        }
    }

    #[test]
    fn factory_rects_initialize_eight_point_anchors_and_offsets_without_changing_opening_positions() {
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0), (1920.0, 1080.0)] {
            let screen = screen(width, height);
            let work = Rect::from_min_max(Pos2::new(8.0, 53.0), Pos2::new(width - 8.0, height - 57.0));
            let people = Rect::from_min_size(super::super::people::default_pos(work), super::super::people::AURORA_SIZE);
            let map = super::super::minimap::default_rect(work);
            let chat = super::super::chat::default_rect(work);
            let (people_point, people_travel) = if height == 720.0 {
                (Align2::LEFT_CENTER, Vec2::new(0.0, 90.0))
            } else {
                (Align2::LEFT_TOP, Vec2::ZERO)
            };
            for (rect, point, travel) in [
                (people, people_point, people_travel),
                (map, Align2::RIGHT_TOP, Vec2::new(320.0, 0.0)),
                (chat, Align2::RIGHT_BOTTOM, Vec2::new(320.0, 180.0)),
            ] {
                let anchor = Anchor::nearest(screen, rect);
                assert_eq!(anchor.point, point, "{width}x{height}: {rect:?}");
                assert_eq!(anchor.position(screen, rect.size()), rect.min);
                let larger = Rect::from_min_size(Pos2::ZERO, Vec2::new(width + 320.0, height + 180.0));
                assert_eq!(anchor.position(larger, rect.size()), rect.min + travel);
            }
        }
    }

    #[test]
    fn an_explicit_placement_is_anchored_after_drawing_and_survives_temporary_clamping() {
        let mut s = Session::new();
        s.frames(3, 1600.0, 900.0);
        crate::ui::widgets::move_floater(&s.ctx, CORNER, Pos2::new(900.0, 70.0));
        s.frames(3, 1600.0, 900.0);
        let rect = s.rect(CORNER);
        assert_eq!(rect.min, Pos2::new(900.0, 70.0));
        let anchor = windows(&s.ctx, |w| {
            w.placed[&Id::unique(CORNER).value()].anchor.expect("explicit placement")
        });
        assert_eq!(anchor.point, egui::Align2::CENTER_TOP);
        s.frames(3, 640.0, 480.0);
        assert_ne!(
            s.rect(CORNER).min,
            anchor.position(move_area(&s.ctx), rect.size()),
            "temporarily clamped"
        );
        assert_eq!(windows(&s.ctx, |w| w.placed[&Id::unique(CORNER).value()].anchor), Some(anchor));
        s.frames(3, 1600.0, 900.0);
        assert_eq!(s.rect(CORNER), rect, "the original offset was retained");
    }

    #[test]
    fn older_layouts_initialize_an_anchor_and_keep_their_rects() {
        #[derive(serde::Serialize)]
        struct LegacyPlaced {
            rect: Rect,
            screen: Rect,
        }
        #[derive(serde::Serialize)]
        struct LegacyWindows {
            placed: BTreeMap<u64, LegacyPlaced>,
        }
        let screen = screen(1280.0, 720.0);
        let rect = Rect::from_min_size(Pos2::new(1000.0, 70.0), Vec2::new(240.0, 200.0));
        let saved = ron::to_string(&LegacyWindows {
            placed: BTreeMap::from([(1, LegacyPlaced { rect, screen })]),
        })
        .expect("legacy serialization");
        let mut windows: Windows = ron::from_str(&saved).expect("backward-compatible layout");
        assert!(windows.placed[&1].anchor.is_none());
        assert_eq!(
            windows.follow(1, screen, default_work_area(screen)),
            Some(rect.min),
            "bar insets must not shift an old layout"
        );
        let anchor = windows.placed[&1].anchor.expect("migration initialized the anchor");
        assert_eq!(anchor.point, egui::Align2::RIGHT_TOP);
        let wider = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        assert_eq!(
            windows.follow(1, wider, default_work_area(wider)),
            Some(rect.min + Vec2::new(320.0, 0.0))
        );
    }

    #[test]
    fn the_allowed_movement_room_drives_anchors_and_clamping_does_not_rewrite_them() {
        let screen = screen(1600.0, 900.0);
        let old = Rect::from_min_max(Pos2::new(40.0, 60.0), Pos2::new(1500.0, 800.0));
        let rect = Rect::from_min_size(Pos2::new(900.0, 570.0), Vec2::new(300.0, 200.0));
        let anchor = Anchor::nearest(old, rect);
        assert_eq!(anchor.point, egui::Align2::CENTER_BOTTOM);
        let mut w = Windows::default();
        w.placed.insert(
            1,
            Placed {
                rect,
                screen,
                room: Some(old),
                anchor: Some(anchor),
            },
        );
        let small = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(400.0, 300.0));
        let asked = w.follow(1, screen, small).expect("room changed without a native resize");
        let constrained = keep_title_in(small, asked, rect.size(), Edges::default());
        assert_ne!(constrained, asked);
        // Model the actual constrained rect being recorded by Slot::shown.
        w.placed.get_mut(&1).expect("placed").rect = Rect::from_min_size(constrained, rect.size());
        w.placed.get_mut(&1).expect("placed").room = Some(small);
        assert_eq!(w.follow(1, screen, old), Some(rect.min));
        assert_eq!(w.placed[&1].anchor, Some(anchor));
    }

    /// A screen too small to lay windows out in (egui itself squeezes them
    /// into it) leaves no trace in the layout: what is recorded is still
    /// where the windows stood in the last real screen. A minimized viewer
    /// window does not even get there: egui-winit keeps the previous screen
    /// rect when the window has no size.
    #[test]
    fn an_unusable_screen_is_not_recorded() {
        let mut s = Session::new();
        s.frames(3, 1600.0, 900.0);
        let before = windows(&s.ctx, |w| w.placed.clone());
        assert_eq!(before.len(), 3);
        assert_eq!(before[&Id::unique(MAP).value()].screen, screen(1600.0, 900.0));
        s.frames(2, 0.0, 0.0);
        s.frames(2, 160.0, 28.0);
        assert_eq!(windows(&s.ctx, |w| w.placed.clone()), before);
    }

    /// The layout saved in a small viewer window, opened in a larger one:
    /// the first frames of the next session put each window back against
    /// its edges, whatever the screen of the very first frame is.
    #[test]
    fn a_saved_layout_follows_the_edges_of_the_next_session() {
        let mut first = Session::new();
        first.frames(3, 1280.0, 720.0);
        let corner = first.gaps(CORNER, 1280.0, 720.0);
        let map = first.gaps(MAP, 1280.0, 720.0);
        let sizes = [first.rect(LEFT).size(), first.rect(CORNER).size(), first.rect(MAP).size()];
        for first_frame in [None, Some((800.0, 600.0)), Some((640.0, 480.0)), Some((2560.0, 1440.0))] {
            let mut next = first.restart();
            if let Some((width, height)) = first_frame {
                next.frame(width, height);
            }
            next.frames(3, 1600.0, 900.0);
            assert_eq!(next.rect(LEFT).min, Pos2::new(8.0, 56.0), "{first_frame:?}");
            let now = next.gaps(CORNER, 1600.0, 900.0);
            assert_eq!((now[2], now[3]), (corner[2], corner[3]), "{first_frame:?}");
            let now = next.gaps(MAP, 1600.0, 900.0);
            assert_eq!((now[1], now[2]), (map[1], map[2]), "{first_frame:?}");
            assert_eq!(
                [next.rect(LEFT).size(), next.rect(CORNER).size(), next.rect(MAP).size()],
                sizes,
                "{first_frame:?}"
            );
        }
    }

    /// A window left smaller than its default size, in the bottom right
    /// corner: the next session, in a larger viewer window where its default
    /// size is larger still (Conversations), it is back in its corner with
    /// its size. egui alone measures it on its first frame and keeps it
    /// inside the screen as if it had its default size, away from the corner.
    #[test]
    fn a_window_smaller_than_its_default_size_keeps_its_corner() {
        let palette = crate::theme::Theme::default().palette();
        let run = |ctx: &egui::Context, frame: usize, width: f32, height: f32, default_height: f32| {
            let input = egui::RawInput {
                screen_rect: Some(screen(width, height)),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let size = Vec2::new(300.0, default_height);
                let pos = Pos2::new(width - 8.0, height - 38.0) - size;
                crate::ui::widgets::Floater::new(CORNER, "Window", pos, size)
                    .silent()
                    .show(ui.ctx(), &palette, &mut true, |ui| {
                        ui.label("content");
                    });
            });
            output.textures_delta.clear();
            ctx.memory(|m| m.area_rect(Id::unique(CORNER))).expect("shown")
        };
        let first = egui::Context::default();
        let mut rect = Rect::NOTHING;
        for frame in 0..3 {
            rect = run(&first, frame, 1280.0, 720.0, 252.0);
        }
        assert_eq!(rect, Rect::from_min_max(Pos2::new(972.0, 430.0), Pos2::new(1272.0, 682.0)));
        let saved = ron::to_string(&first.memory(|m| m.clone())).expect("serialize");
        // the same viewer window, then a larger one
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0)] {
            let next = egui::Context::default();
            restore(&next, &saved).expect("restore");
            for frame in 0..4 {
                rect = run(&next, frame, width, height, 432.0);
            }
            assert_eq!(rect.size(), Vec2::new(300.0, 252.0), "{width} x {height}");
            assert_eq!(rect.right_bottom(), Pos2::new(width - 8.0, height - 38.0), "{width} x {height}");
            let recorded = windows(&next, |w| w.placed[&Id::unique(CORNER).value()]);
            assert_eq!((recorded.rect, recorded.screen), (rect, screen(width, height)));
        }
    }

    /// A window closed while the viewer window changed size follows its
    /// edges when it opens again.
    #[test]
    fn a_window_reopened_after_a_resize_follows_its_edges() {
        let ctx = egui::Context::default();
        let palette = crate::theme::Theme::default().palette();
        let mut time = 0.0;
        let mut frame = |width: f32, open: bool| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(screen(width, 720.0)),
                time: Some(time),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                crate::ui::widgets::Floater::new(CORNER, "Window", Pos2::new(972.0, 100.0), Vec2::new(300.0, 200.0))
                    .silent()
                    .show(ui.ctx(), &palette, &mut { open }, |ui| {
                        ui.label("content");
                    });
            });
            output.textures_delta.clear();
        };
        frame(1280.0, true);
        frame(1280.0, true);
        frame(1600.0, false);
        frame(1600.0, false);
        frame(1600.0, true);
        frame(1600.0, true);
        let rect = ctx.memory(|m| m.area_rect(Id::unique(CORNER))).expect("shown");
        assert_eq!(1600.0 - rect.right(), 8.0, "{rect:?}");
    }

    #[test]
    fn layout_round_trip_keeps_window_rects() {
        let ctx = egui::Context::default();
        let run = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new("t")
                    .id(egui::Id::unique("floater_t"))
                    .default_pos([123.0, 45.0])
                    .show(ui.ctx(), |ui| {
                        ui.label("x");
                    });
                ui.ctx().data_mut(|d| d.insert_persisted(egui::Id::unique("min"), true));
            });
            out.textures_delta.clear();
        };
        run(&ctx);
        let text = ron::to_string(&ctx.memory(|m| m.clone())).expect("serialize");
        let back: egui::Memory = ron::from_str(&text).expect("deserialize");
        let ctx2 = egui::Context::default();
        ctx2.memory_mut(|m| *m = back);
        assert_eq!(ctx2.data_mut(|d| d.get_persisted::<bool>(egui::Id::unique("min"))), Some(true));
        let rect = ctx2.memory(|m| m.area_rect(egui::Id::unique("floater_t")));
        assert!(rect.is_some_and(|r| (r.min.x - 123.0).abs() < 1.0), "{rect:?}");
    }

    /// The order of `App::new`: the interface font chosen in Preferences is
    /// installed after the saved layout. The other way round, a session with
    /// a saved layout started with egui's own fonts (and without the system
    /// fallback faces, #127), whatever the setting.
    #[test]
    fn fonts_installed_after_the_layout_are_kept() {
        let saved = ron::to_string(&egui::Context::default().memory(|m| m.clone())).expect("serialize");
        let first_font = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
            ctx.fonts(|f| f.definitions().families[&egui::FontFamily::Proportional][0].clone())
        };
        let ctx = egui::Context::default();
        restore(&ctx, &saved).expect("restore");
        crate::theme::Theme::default().apply(&ctx, 1.4);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        assert_eq!(first_font(&ctx), "aurora-main");
        assert_eq!(crate::ui::fonts::scale(&ctx), 1.4);
        ctx.fonts(|fonts| assert_eq!(fonts.definitions().font_data["aurora-main"].tweak.scale, 1.4));

        let ctx = egui::Context::default();
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        super::restore(&ctx, &saved).expect("restore");
        assert_ne!(first_font(&ctx), "aurora-main", "egui keeps pending fonts now: the order is free");
    }

    const ALL: Edges = Edges {
        left: true,
        right: true,
        bottom: true,
    };
    const NONE: Edges = Edges {
        left: false,
        right: false,
        bottom: false,
    };

    /// By default every edge stops a window: it stays whole on the screen,
    /// anywhere on it, never above the top.
    #[test]
    fn by_default_a_window_stays_whole_on_the_screen() {
        let room = screen(1280.0, 720.0);
        let size = Vec2::new(400.0, 300.0);
        assert_eq!(Edges::default(), ALL);
        assert_eq!(reach(room, size, ALL), room);
        let keep = |x: f32, y: f32| keep_title_in(room, Pos2::new(x, y), size, ALL);
        assert_eq!(keep(5000.0, 5000.0), Pos2::new(880.0, 420.0));
        assert_eq!(keep(-5000.0, -5000.0), Pos2::new(0.0, 0.0));
        assert_eq!(keep(300.0, 200.0), Pos2::new(300.0, 200.0));
    }

    /// An edge let open: a window crosses it, by the sides but `TITLE_GRIP`
    /// of its title bar, by the bottom but its whole title bar; the other
    /// edges still stop it, the top always does.
    #[test]
    fn an_open_edge_lets_a_window_cross_it_within_reach() {
        let room = screen(1280.0, 720.0);
        let size = Vec2::new(400.0, 300.0);
        let fits = |edges: Edges, pos: Pos2| reach(room, size, edges).contains_rect(Rect::from_min_size(pos, size));
        let only = |left, right, bottom| Edges { left, right, bottom };
        // right open
        assert!(fits(only(true, false, true), Pos2::new(1280.0 - TITLE_GRIP, 300.0)));
        assert!(!fits(only(true, false, true), Pos2::new(1281.0 - TITLE_GRIP, 300.0)));
        assert!(!fits(only(true, false, true), Pos2::new(-1.0, 300.0)), "the left still stops it");
        assert!(!fits(only(true, false, true), Pos2::new(300.0, 421.0)), "the bottom still stops it");
        // left open
        assert!(fits(only(false, true, true), Pos2::new(-(400.0 - TITLE_GRIP), 300.0)));
        assert!(!fits(only(false, true, true), Pos2::new(-(401.0 - TITLE_GRIP), 300.0)));
        assert!(!fits(only(false, true, true), Pos2::new(881.0, 300.0)), "the right still stops it");
        // bottom open: down to its title bar
        assert!(fits(only(true, true, false), Pos2::new(300.0, 720.0 - TITLE_BAR_H)));
        assert!(!fits(only(true, true, false), Pos2::new(300.0, 721.0 - TITLE_BAR_H)));
        // all open: never above the top
        assert!(!fits(NONE, Pos2::new(300.0, -1.0)));
        // a saved place comes back by the same rules
        for edges in [ALL, NONE, only(false, true, true), only(true, false, false)] {
            for pos in [Pos2::new(5000.0, -300.0), Pos2::new(-5000.0, 5000.0), Pos2::new(640.0, 360.0)] {
                assert!(fits(edges, keep_title_in(room, pos, size, edges)), "{edges:?} {pos:?}");
            }
        }
        // a window narrower than the grip never crosses a side
        assert_eq!(reach(room, Vec2::new(60.0, TITLE_BAR_H), NONE), room);
    }

    /// Islands of the top row at 8..44 px: one 600 px wide on the left, one
    /// 200 px wide on the right, a gap between them.
    fn top_islands() -> Vec<Rect> {
        vec![
            Rect::from_min_max(Pos2::new(8.0, 8.0), Pos2::new(608.0, 44.0)),
            Rect::from_min_max(Pos2::new(1072.0, 8.0), Pos2::new(1272.0, 44.0)),
        ]
    }

    /// A window can sit in the gap between two islands of the top row; one
    /// whose title would be hidden under an island comes out below the row,
    /// one under the bottom islands above them, the place it had across.
    #[test]
    fn a_title_left_under_the_islands_comes_out_of_them() {
        let room = screen(1280.0, 720.0);
        let islands = top_islands();
        let window = Vec2::new(300.0, 200.0);
        // in the gap, its title over the row: left as it is
        let gap = Pos2::new(700.0, 10.0);
        assert_eq!(clear_of_islands(room, gap, window, ALL, &islands), gap);
        // straddling an island but with a grip in the gap: left as it is
        let straddle = Pos2::new(500.0, 10.0);
        assert_eq!(clear_of_islands(room, straddle, window, ALL, &islands), straddle);
        // hidden under the left island: below the row, 8 px under it
        let hidden = Pos2::new(100.0, 10.0);
        assert_eq!(clear_of_islands(room, hidden, window, ALL, &islands), Pos2::new(100.0, 52.0));
        // a minimized window wider than the room left beside the island
        let minimized = Vec2::new(180.0, TITLE_BAR_H);
        assert_eq!(clear_of_islands(room, Pos2::new(1080.0, 20.0), minimized, ALL, &islands).y, 52.0);
        // a minimized one under the dock at the bottom: above it
        let dock = [Rect::from_min_max(Pos2::new(400.0, 668.0), Pos2::new(880.0, 712.0))];
        assert_eq!(
            clear_of_islands(room, Pos2::new(500.0, 670.0), minimized, ALL, &dock),
            Pos2::new(500.0, 668.0 - 8.0 - TITLE_BAR_H)
        );
        // a window let past the bottom, its title on the dock: above it too
        assert_eq!(
            clear_of_islands(room, Pos2::new(500.0, 670.0), window, NONE, &dock),
            Pos2::new(500.0, 668.0 - 8.0 - TITLE_BAR_H)
        );
        // no island: nothing moves
        assert_eq!(clear_of_islands(room, hidden, window, ALL, &[]), hidden);
    }

    /// What shows of a title bar beside the islands crossing its height.
    #[test]
    fn the_uncovered_width_of_a_title_bar() {
        let room = screen(1280.0, 720.0);
        let islands = top_islands();
        let title = |x: f32, y: f32, w: f32| Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, TITLE_BAR_H));
        assert_eq!(uncovered(title(700.0, 10.0, 300.0), room, &islands), 300.0);
        assert_eq!(uncovered(title(500.0, 10.0, 300.0), room, &islands), 192.0);
        assert_eq!(uncovered(title(100.0, 10.0, 300.0), room, &islands), 0.0);
        // below the row: nothing hides it
        assert_eq!(uncovered(title(100.0, 60.0, 300.0), room, &islands), 300.0);
        // partly off the screen: only what is on it counts
        assert_eq!(uncovered(title(1200.0, 300.0, 300.0), room, &islands), 80.0);
    }

    /// The save waits for the layout to stay still, each change pushing it
    /// back, and comes once.
    #[test]
    fn the_autosave_waits_for_the_layout_to_settle() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut autosave = Autosave::default();
        assert!(!autosave.mark(false, at(0)), "nothing changed, nothing to save");
        assert!(!autosave.mark(true, at(0)));
        assert!(!autosave.mark(false, at(1_000)));
        // moved again: the delay starts over
        assert!(!autosave.mark(true, at(1_500)));
        assert!(!autosave.mark(false, at(3_000)));
        assert!(autosave.mark(false, at(3_500)));
        assert!(!autosave.mark(false, at(10_000)), "saved once");
    }

    /// A layout with the window `id` at `x`.
    fn layout_at(id: &str, x: f32) -> egui::Memory {
        let ctx = egui::Context::default();
        // the second pass records the window's place
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new(id)
                    .id(Id::unique(id))
                    .default_pos(Pos2::new(x, 50.0))
                    .show(ui.ctx(), |ui| ui.label("x"));
            });
            out.textures_delta.clear();
        }
        ctx.memory(|m| m.clone())
    }

    fn saved_x(p: &Path, id: &str) -> f32 {
        let saved = std::fs::read_to_string(p).expect("saved layout");
        let ctx = egui::Context::default();
        restore(&ctx, &saved).expect("readable layout");
        ctx.memory(|m| m.area_rect(Id::unique(id))).expect("saved window").min.x
    }

    /// A background save that ends after a newer one (the exit's) or after a
    /// reset never brings its older layout back.
    #[test]
    fn an_older_layout_never_replaces_a_newer_one() {
        let dir = crate::settings::config_dir().join(format!("layout-autosave-{}", uuid::Uuid::new_v4()));
        let p = dir.join("ui_layout.ron");
        let state = Mutex::new(Written { generation: 0, text: None });
        let moved = layout_at("autosave", 300.0);
        write_to(&state, &p, 2, &moved);
        write_to(&state, &p, 1, &layout_at("autosave", 100.0));
        assert_eq!(saved_x(&p, "autosave"), 300.0);
        // the same layout again: the file is left as it is
        let modified = std::fs::metadata(&p).and_then(|m| m.modified()).expect("modified");
        write_to(&state, &p, 3, &moved);
        assert_eq!(std::fs::metadata(&p).and_then(|m| m.modified()).expect("modified"), modified);
        // reset, then a save taken before it arrives
        discard(&state, &p, 5);
        write_to(&state, &p, 4, &layout_at("autosave", 100.0));
        assert!(!p.exists(), "the reset layout stays forgotten");
        write_to(&state, &p, 6, &layout_at("autosave", 200.0));
        assert_eq!(saved_x(&p, "autosave"), 200.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
