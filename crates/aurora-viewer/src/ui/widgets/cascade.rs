//! Port of LLFloaterReg::getLastFloaterInGroup, LLFloater::applyRectControl
//! and stackWith (Firestorm indra/llui/{llfloaterreg,llfloater}.cpp, LGPL 2.1).
//! Instance order, not keyboard focus, chooses the visible group anchor.

use egui::{Context, Id, Pos2, Rect, Vec2};

/// Firestorm registers each preview separately; inventory and live script
/// editors alone share the same `preview_script` group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    Texture,
    Material,
    Notecard,
    Script,
    Animation,
    Sound,
    Gesture,
    /// A conversation in its own window (floater_fs_im_session.xml).
    Conversation,
    /// Properties of an object's contents item: the Legacy Viewer's
    /// per-item "properties" instances stack in one group.
    TaskProperties,
    /// A group's profile (FSFloaterGroup, one instance per group), in the
    /// classic or the Aurora look: their sizes differ.
    Profile {
        classic: bool,
    },
    Inventory(i32),
}

impl Group {
    pub(crate) fn inventory(kind: i32) -> Self {
        match kind {
            0 => Self::Texture,
            1 => Self::Sound,
            7 => Self::Notecard,
            10 => Self::Script,
            20 => Self::Animation,
            21 => Self::Gesture,
            57 => Self::Material,
            kind => Self::Inventory(kind),
        }
    }

    fn saves_rect(self) -> bool {
        // The corresponding Firestorm XUI files use save_rect="true".
        // Resource sizes have their own human-requested preferences.
        matches!(
            self,
            Self::Notecard | Self::Script | Self::Animation | Self::Sound | Self::Gesture | Self::Conversation | Self::Profile { .. }
        )
    }

    /// A window stacked on another one keeps its step down and right
    /// even when that leaves it less room than its size, getting smaller
    /// instead of being pushed back over the window below (human request,
    /// #4788: the group profiles, taller than most screens' room). Visible
    /// difference: the Legacy Viewer's adjustToFitScreen moves it back.
    fn keeps_step(self) -> bool {
        matches!(self, Self::Profile { .. })
    }

    fn saved_id(self) -> Id {
        Id::unique(("inventory_cascade_rect", self))
    }
}

const OFFSET: Vec2 = Vec2::splat(32.0); // Human-requested double of Firestorm's 16 px UIFloaterOffset

/// A `keeps_step` window shrinks at most to this part of the group's first
/// window, then the stack starts a new column (Aurora's: the Legacy Viewer
/// piles them against the screen's edge).
const WRAP_LEAST: f32 = 0.5;

#[derive(Clone)]
struct Window {
    id: Id,
    group: Group,
    rect: Rect,
    frame: u64,
    born: u64,
    opening: bool,
    target: Pos2,
    /// The height a `keeps_step` window was made smaller to, when it was.
    asked: Option<f32>,
    primary: bool,
    visible: bool,
    minimized: bool,
}

#[derive(Clone, Default)]
struct Windows(Vec<Window>);

fn registry_id() -> Id {
    Id::unique("inventory_cascade_windows")
}

pub(super) struct Opening {
    pub pos: Pos2,
    pub size: Option<Vec2>,
}

/// Called only by opted-in floaters. A remembered per-item egui position
/// must not override a new instance's group placement. Existing instances
/// keep their position, including when raised/restored or the UI is hidden.
/// `least` is the floater's least size, under which it is not made smaller.
pub(super) fn prepare(ctx: &Context, id: Id, group: Group, default: Rect, work: Rect, least: Vec2) -> Option<Opening> {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|data| {
        let saved = group.saves_rect().then(|| data.get_persisted::<Rect>(group.saved_id())).flatten();
        let windows = data.get_temp_mut_or_default::<Windows>(registry_id());
        windows.0.retain(|w| w.frame.saturating_add(1) >= frame);
        if let Some(w) = windows.0.iter_mut().find(|w| w.id == id) {
            // egui first measures a new window. Fit its actual outer size on
            // the following frame too, without repositioning older windows.
            return (w.opening && frame <= w.born + 1).then(|| Opening {
                pos: super::fit_in(work, w.target, w.rect.size()),
                size: None,
            });
        }
        let anchor = windows.0.iter().rev().find(|w| w.group == group && w.visible && !w.minimized);
        let primary = anchor.is_none();
        let mut size = saved.map_or(default.size(), |r| r.size()).min(work.size());
        let mut target = anchor.map_or_else(|| saved.map_or(default.min, |r| r.min), |w| w.rect.min + OFFSET);
        // a step down that would leave less than half the group's first
        // window, or less than a window of the group can really take, starts
        // a new column, back at that window's top; at the screen's right edge
        // the windows stop moving right and keep their width, stepping down
        // only (human request, #4788: past ~13 group profiles the shrunk
        // windows ended up anywhere, then piled at one spot)
        if anchor.is_some()
            && group.keeps_step()
            && let Some(first) = windows.0.iter().find(|w| w.group == group && w.visible && !w.minimized)
        {
            if work.max.y - target.y < column_floor(&windows.0, group, first.rect.height(), least.y) {
                target.y = first.rect.min.y;
            }
            target.x = target.x.min(work.max.x - size.x).max(work.min.x);
        }
        let room = (work.max - target).max(Vec2::ZERO);
        let shrunk = !primary && group.keeps_step() && (size.x > room.x || size.y > room.y);
        if shrunk {
            size = size.min(room);
        }
        let pos = super::fit_in(work, target, size);
        windows.0.push(Window {
            id,
            group,
            rect: Rect::from_min_size(pos, size),
            frame,
            born: frame,
            opening: true,
            target,
            asked: shrunk.then_some(size.y),
            primary,
            visible: true,
            minimized: false,
        });
        Some(Opening {
            pos,
            size: (saved.is_some() || shrunk).then_some(size),
        })
    })
}

/// The least room a stacked `keeps_step` window needs below its step: half
/// the group's first window, the floater's least height, and the height a
/// window made smaller really took when its content kept it taller (it was
/// then put back inside, over the one before it, and every later one would
/// land at that same spot).
fn column_floor(windows: &[Window], group: Group, first: f32, least: f32) -> f32 {
    windows
        .iter()
        .filter(|w| w.group == group && w.visible && !w.minimized)
        .filter_map(|w| w.asked.filter(|&asked| w.rect.height() > asked + 1.0).map(|_| w.rect.height()))
        .fold((first * WRAP_LEAST).max(least), f32::max)
}

pub(super) fn shown(ctx: &Context, id: Id, rect: Rect, minimized: bool) {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|data| {
        let saved = {
            let windows = data.get_temp_mut_or_default::<Windows>(registry_id());
            let Some(w) = windows.0.iter_mut().find(|w| w.id == id) else {
                return;
            };
            w.frame = frame;
            w.rect = rect;
            w.visible = true;
            w.minimized = minimized;
            if frame > w.born {
                w.opening = false;
            }
            (w.primary && !minimized && w.group.saves_rect()).then_some((w.group.saved_id(), rect))
        };
        if let Some((key, rect)) = saved {
            data.insert_persisted(key, rect);
        }
    });
}

pub(super) fn hidden(ctx: &Context, id: Id) {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|data| {
        let windows = data.get_temp_mut_or_default::<Windows>(registry_id());
        if let Some(w) = windows.0.iter_mut().find(|w| w.id == id) {
            w.visible = false;
            w.frame = frame;
        }
    });
}

pub(super) fn minimized(ctx: &Context, id: Id, minimized: bool) {
    ctx.data_mut(|data| {
        let windows = data.get_temp_mut_or_default::<Windows>(registry_id());
        if let Some(w) = windows.0.iter_mut().find(|w| w.id == id) {
            w.minimized = minimized;
        }
    });
}

pub(super) fn forget(ctx: &Context, id: Id) {
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Windows>(registry_id()).0.retain(|w| w.id != id));
}

/// "Reset window positions and sizes": the groups that remember where
/// their first window stood (`Group::saves_rect`) forget it.
pub(super) fn forget_saved(ctx: &Context) {
    use Group::*;
    ctx.data_mut(|data| {
        let profiles = [Profile { classic: false }, Profile { classic: true }];
        for group in [Notecard, Script, Animation, Sound, Gesture, Conversation]
            .into_iter()
            .chain(profiles)
        {
            debug_assert!(group.saves_rect());
            data.remove::<Rect>(group.saved_id());
        }
    });
}

pub(super) fn placed(ctx: &Context, id: Id) {
    ctx.data_mut(|data| {
        let windows = data.get_temp_mut_or_default::<Windows>(registry_id());
        if let Some(w) = windows.0.iter_mut().find(|w| w.id == id) {
            w.opening = false;
        }
    });
}
