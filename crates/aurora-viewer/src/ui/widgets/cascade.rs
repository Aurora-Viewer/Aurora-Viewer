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
            Self::Notecard | Self::Script | Self::Animation | Self::Sound | Self::Gesture | Self::Conversation
        )
    }

    fn saved_id(self) -> Id {
        Id::unique(("inventory_cascade_rect", self))
    }
}

const OFFSET: Vec2 = Vec2::splat(32.0); // Human-requested double of Firestorm's 16 px UIFloaterOffset

#[derive(Clone)]
struct Window {
    id: Id,
    group: Group,
    rect: Rect,
    frame: u64,
    born: u64,
    opening: bool,
    target: Pos2,
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
pub(super) fn prepare(ctx: &Context, id: Id, group: Group, default: Rect, work: Rect) -> Option<Opening> {
    let frame = ctx.cumulative_frame_nr();
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
        let size = saved.map_or(default.size(), |r| r.size()).min(work.size());
        let target = anchor.map_or_else(|| saved.map_or(default.min, |r| r.min), |w| w.rect.min + OFFSET);
        let pos = super::fit_in(work, target, size);
        windows.0.push(Window {
            id,
            group,
            rect: Rect::from_min_size(pos, size),
            frame,
            born: frame,
            opening: true,
            target,
            primary,
            visible: true,
            minimized: false,
        });
        Some(Opening {
            pos,
            size: saved.map(|_| size),
        })
    })
}

pub(super) fn shown(ctx: &Context, id: Id, rect: Rect, minimized: bool) {
    let frame = ctx.cumulative_frame_nr();
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
    let frame = ctx.cumulative_frame_nr();
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

pub(super) fn placed(ctx: &Context, id: Id) {
    ctx.data_mut(|data| {
        let windows = data.get_temp_mut_or_default::<Windows>(registry_id());
        if let Some(w) = windows.0.iter_mut().find(|w| w.id == id) {
            w.opening = false;
        }
    });
}
