//! UI layout kept between sessions (Firestorm saves floater rects): egui's
//! memory (window positions and sizes, collapsed floaters) is written to
//! `ui_layout.ron` on exit and restored at startup. Which panels are open
//! lives in the settings.
//!
//! egui remembers the top left corner of a window. When the viewer window
//! changes size, live or between two sessions, each window keeps instead its
//! distance to the screen edges it is nearest to (`follow_edges`): the
//! mini-map stays in its top right corner, what stands on the left stays on
//! the left. Every window the user places goes through a `Slot` for that
//! (`widgets::Floater`, the mini-map): the slot records where the window
//! stood and in which screen, with the layout.
//!
//! The slots also make the windows of the frame known to the gestures
//! (`gesture`): a window moved or resized snaps to the others and to the
//! work area (`snap`), and windows snapped side by side are glued: they
//! move as one block, saved with the layout too.

mod gesture;

use super::snap;
use crate::theme::Palette;
use egui::{Id, Pos2, Rect, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn path() -> PathBuf {
    crate::settings::config_dir().join("ui_layout.ron")
}

/// Restore the saved layout. The style and options set by the skin are
/// kept (only positions, sizes and per-window state come from the file).
///
/// Call it before `fonts::apply`: egui keeps the font definitions that wait
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

pub fn save(ctx: &egui::Context) {
    let mem = ctx.memory(|m| m.clone());
    match ron::to_string(&mem) {
        Ok(text) => {
            let p = path();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // write then rename: never leave a half-written file
            let tmp = p.with_extension("ron.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
        Err(e) => log::warn!("ui layout not saved: {e}"),
    }
}

/// Forget the saved layout ("Reset everything").
pub fn forget(ctx: &egui::Context) {
    let _ = std::fs::remove_file(path());
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = egui::Memory::default());
    ctx.options_mut(|o| *o = options);
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
    Id::new("layout_work_area")
}

/// The room left by the bars this frame: call it once they are drawn,
/// before the windows are shown.
pub(crate) fn set_work_area(ctx: &egui::Context, area: Rect) {
    ctx.data_mut(|d| d.insert_temp(work_area_id(), area));
}

/// Where the windows are laid out: the screen without the bars.
pub(crate) fn work_area(ctx: &egui::Context) -> Rect {
    ctx.data(|d| d.get_temp::<Rect>(work_area_id()))
        .unwrap_or_else(|| default_work_area(ctx.content_rect()))
}

/// A screen smaller than this is not one to lay windows out in: the content
/// rect of a minimized viewer window, or of the first frames before the
/// surface has its size. Windows neither follow it nor record it.
const MIN_SCREEN: Vec2 = Vec2::new(320.0, 240.0);

fn usable(screen: Rect) -> bool {
    screen.is_finite() && screen.width() >= MIN_SCREEN.x && screen.height() >= MIN_SCREEN.y
}

/// How far `window` moves when the screen it stood in goes from `old` to
/// `new`: it keeps its distance to the nearer of the left and right edges,
/// and to the nearer of the top and bottom ones, as Firestorm's floaters
/// follow the edge they are closest to when the viewer window is resized
/// (LLFloaterView::reshape, indra/llui/llfloater.cpp). At equal distances
/// (a centered window) it stays with the left and top edges. An unusable
/// screen on either side moves nothing.
pub(crate) fn follow_edges(old: Rect, new: Rect, window: Rect) -> Vec2 {
    if !usable(old) || !usable(new) {
        return Vec2::ZERO;
    }
    let near_right = old.right() - window.right() < window.left() - old.left();
    let near_bottom = old.bottom() - window.bottom() < window.top() - old.top();
    Vec2::new(
        if near_right {
            new.right() - old.right()
        } else {
            new.left() - old.left()
        },
        if near_bottom {
            new.bottom() - old.bottom()
        } else {
            new.top() - old.top()
        },
    )
}

/// Where a window stood the last time it was shown, and in which screen.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Placed {
    rect: Rect,
    screen: Rect,
}

/// A window shown this frame or the last one.
#[derive(Clone, Copy, Debug)]
struct Shown {
    id: Id,
    rect: Rect,
    /// The frame (`Live::frame`) it was last shown in.
    frame: u64,
    minimized: bool,
}

/// What is known of the windows for the session only.
#[derive(Clone, Debug, Default)]
struct Live {
    /// Frames counted by `frame` (the gestures); stays 0 without it.
    frame: u64,
    shown: BTreeMap<u64, Shown>,
    /// The screen the targets of `follow` were computed for, and where each
    /// window recorded in another screen goes in it.
    followed: Option<Rect>,
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
    /// Where the windows recorded in another screen go in `screen`: each
    /// follows its edges, a group of glued windows as one block (the edges
    /// nearest to its bounds), so that its windows stay side by side.
    /// Computed once per screen, before any of them is recorded in it.
    fn follow(&mut self, window: u64, screen: Rect) -> Option<Pos2> {
        if self.live.followed != Some(screen) {
            self.live.followed = Some(screen);
            self.live.follow = self
                .placed
                .iter()
                .filter(|(_, placed)| placed.screen != screen)
                .map(|(id, placed)| {
                    let block = snap::group(&self.links, *id)
                        .iter()
                        .filter_map(|member| self.placed.get(member))
                        .filter(|member| member.screen == placed.screen)
                        .fold(Rect::NOTHING, |all, member| all.union(member.rect));
                    (*id, placed.rect.min + follow_edges(placed.screen, screen, block))
                })
                .collect();
        }
        self.live.follow.remove(&window)
    }
}

fn windows<R>(ctx: &egui::Context, read: impl FnOnce(&mut Windows) -> R) -> R {
    ctx.data_mut(|d| read(d.get_persisted_mut_or_default::<Windows>(Id::new("window_layout"))))
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
    let sized = egui::AreaState::load(ctx, id).is_some_and(|s| s.size.is_some());
    let key = id.value();
    let (pos, limits) = windows(ctx, |w| {
        let followed = usable(screen).then(|| w.follow(key, screen)).flatten();
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
        (pos, limits)
    });
    Slot { id, sized, pos, limits }
}

impl Slot {
    /// The window was shown at `rect`; `minimized`: reduced to its title.
    pub(crate) fn shown(self, ctx: &egui::Context, rect: Rect, minimized: bool) {
        let screen = ctx.content_rect();
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
            let new = Placed { rect, screen };
            if usable(screen) && (self.sized || recorded.is_some()) && recorded != Some(new) {
                w.placed.insert(key, new);
            }
            let frame = w.live.frame;
            w.live.shown.insert(
                key,
                Shown {
                    id: self.id,
                    rect,
                    frame,
                    minimized,
                },
            );
        });
    }
}

/// The gestures of the frame on the windows: call it once per frame, after
/// the bars (`set_work_area`) and before the windows are shown. A window
/// dragged by its title takes the windows glued to it along, is offered a
/// snap to the others and to the work area (`snap`; `snap_enabled` is the
/// user's setting), shown by a guide and applied when the mouse is
/// released; Alt moves it alone and free.
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
    let (area, pixels_per_point) = (work_area(ctx), ctx.pixels_per_point());
    let overlay = windows(ctx, |w| w.drive(area, pixels_per_point, pointer, snap_enabled));
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

    /// A 200 x 100 window whose top left corner is at (x, y).
    fn window(x: f32, y: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(200.0, 100.0))
    }

    #[test]
    fn a_window_near_the_right_edge_keeps_its_gap_to_it() {
        let old = screen(1280.0, 720.0);
        let map = window(1064.0, 56.0);
        for width in [1600.0, 1920.0, 1000.0] {
            let new = screen(width, 720.0);
            let moved = map.translate(follow_edges(old, new, map));
            assert_eq!(new.right() - moved.right(), 16.0, "{width}");
            assert_eq!(moved.top(), 56.0, "{width}");
        }
    }

    #[test]
    fn a_window_near_the_left_or_top_edge_does_not_move() {
        let old = screen(1280.0, 720.0);
        for new in [screen(1600.0, 900.0), screen(1024.0, 600.0)] {
            assert_eq!(follow_edges(old, new, window(8.0, 56.0)), Vec2::ZERO);
        }
    }

    #[test]
    fn a_window_near_the_bottom_edge_follows_it() {
        let old = screen(1280.0, 720.0);
        let chat = window(8.0, 580.0);
        for height in [900.0, 600.0] {
            let new = screen(1280.0, height);
            let moved = chat.translate(follow_edges(old, new, chat));
            assert_eq!(new.bottom() - moved.bottom(), 40.0, "{height}");
            assert_eq!(moved.left(), 8.0);
        }
        // the bottom right corner follows both
        let corner = window(1072.0, 580.0);
        let new = screen(1600.0, 900.0);
        let moved = corner.translate(follow_edges(old, new, corner));
        assert_eq!((new.right() - moved.right(), new.bottom() - moved.bottom()), (8.0, 40.0));
    }

    /// At the same distance from both edges, the left and the top ones win:
    /// a centered window stays where it is, on each axis.
    #[test]
    fn a_centered_window_stays_put() {
        let old = screen(1280.0, 720.0);
        let centered = window(540.0, 310.0);
        assert_eq!(follow_edges(old, screen(1600.0, 900.0), centered), Vec2::ZERO);
        // one pixel nearer the right edge, it follows that one
        let right = window(541.0, 310.0);
        assert_eq!(follow_edges(old, screen(1600.0, 900.0), right), Vec2::new(320.0, 0.0));
    }

    #[test]
    fn nothing_moves_in_an_unchanged_or_unusable_screen() {
        let old = screen(1280.0, 720.0);
        let map = window(1064.0, 56.0);
        assert_eq!(follow_edges(old, old, map), Vec2::ZERO);
        // minimized viewer window, first frames
        for tiny in [screen(0.0, 0.0), screen(120.0, 28.0), screen(1280.0, 0.0), screen(f32::NAN, 720.0)] {
            assert!(!usable(tiny));
            assert_eq!(follow_edges(old, tiny, map), Vec2::ZERO);
            assert_eq!(follow_edges(tiny, old, map), Vec2::ZERO);
        }
    }

    /// The origin of the screen moves too when a side bar takes room.
    #[test]
    fn edges_are_those_of_the_screen_rect() {
        let old = screen(1280.0, 720.0);
        let new = Rect::from_min_max(Pos2::new(40.0, 20.0), Pos2::new(1640.0, 920.0));
        assert_eq!(follow_edges(old, new, window(8.0, 56.0)), Vec2::new(40.0, 20.0));
        assert_eq!(follow_edges(old, new, window(1064.0, 580.0)), Vec2::new(360.0, 200.0));
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
                let floater = |id, pos| {
                    crate::ui::widgets::Floater::new(id, "Window", pos, Vec2::new(300.0, 200.0))
                        .silent()
                        .show(ctx, &palette, &mut true, |ui| {
                            ui.label("content");
                        });
                };
                floater(LEFT, Pos2::new(8.0, 56.0));
                floater(CORNER, Pos2::new(972.0, 480.0));
                let slot = slot(ctx, Id::new(MAP));
                let mut map = egui::Window::new("Map")
                    .id(Id::new(MAP))
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
            self.ctx.memory(|m| m.area_rect(Id::new(id))).expect("the window was shown")
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
        assert_eq!(before[&Id::new(MAP).value()].screen, screen(1600.0, 900.0));
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
            ctx.memory(|m| m.area_rect(Id::new(CORNER))).expect("shown")
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
            let recorded = windows(&next, |w| w.placed[&Id::new(CORNER).value()]);
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
        let rect = ctx.memory(|m| m.area_rect(Id::new(CORNER))).expect("shown");
        assert_eq!(1600.0 - rect.right(), 8.0, "{rect:?}");
    }

    #[test]
    fn layout_round_trip_keeps_window_rects() {
        let ctx = egui::Context::default();
        let run = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new("t")
                    .id(egui::Id::new("floater_t"))
                    .default_pos([123.0, 45.0])
                    .show(ui.ctx(), |ui| {
                        ui.label("x");
                    });
                ui.ctx().data_mut(|d| d.insert_persisted(egui::Id::new("min"), true));
            });
            out.textures_delta.clear();
        };
        run(&ctx);
        let text = ron::to_string(&ctx.memory(|m| m.clone())).expect("serialize");
        let back: egui::Memory = ron::from_str(&text).expect("deserialize");
        let ctx2 = egui::Context::default();
        ctx2.memory_mut(|m| *m = back);
        assert_eq!(ctx2.data_mut(|d| d.get_persisted::<bool>(egui::Id::new("min"))), Some(true));
        let rect = ctx2.memory(|m| m.area_rect(egui::Id::new("floater_t")));
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
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::NOTO);
        assert_eq!(first_font(&ctx), "aurora-main");

        let ctx = egui::Context::default();
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::NOTO);
        super::restore(&ctx, &saved).expect("restore");
        assert_ne!(first_font(&ctx), "aurora-main", "egui keeps pending fonts now: the order is free");
    }
}
