//! The World filters window (Preferences › Graphics › Image › World
//! filters): the world's own stack of looks (`snapshot::looks::WorldLook`),
//! edited with the photo's looks editor and drawn on the 3D view, under the
//! interface, outside the photo mode and the Director. Aurora's own: the
//! Legacy Viewer has filters only in its snapshot window.

use super::looks_editor::{LooksUi, Options, Stack};
use super::snapshot::Action;
use super::widgets;
use crate::snapshot::looks::{Layer, WorldLook};
use crate::theme::Palette;
use egui::{RichText, Vec2};

pub const WINDOW: &str = "world_look";

/// The window's state for the session.
#[derive(Debug, Default)]
pub struct WorldLookUi {
    pub looks: LooksUi,
    /// A table imported from this window goes into the world's stack.
    pub importing: bool,
}

/// Draw the window. `photo_shown`: the photo window or the Director is
/// open and draws its own looks instead.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    open: &mut bool,
    state: &mut WorldLookUi,
    look: &mut WorldLook,
    luts: &[String],
    photo_shown: bool,
    out: &mut Vec<Action>,
) {
    widgets::Floater::new(
        WINDOW,
        aurora_i18n::tr!("world-look-title"),
        egui::pos2(460.0, 110.0),
        Vec2::new(340.0, 620.0),
    )
    .help(aurora_i18n::tr!("world-look-help"))
    .show(ctx, p, open, |ui| {
        widgets::switch_labeled(ui, p, &mut look.on, aurora_i18n::tr!("world-look-on"));
        let note = if photo_shown {
            aurora_i18n::tr!("world-look-photo-open")
        } else {
            aurora_i18n::tr!("world-look-note")
        };
        ui.add(egui::Label::new(RichText::new(note).size(11.5).color(p.muted)).wrap());
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("world_look_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let before = look.looks.clone();
                let opts = Options {
                    list_height: 260.0,
                    compare: false,
                    luts,
                    pano: false,
                };
                let mut actions = Vec::new();
                state.looks.show(ui, p, Stack::world(look), opts, &mut actions);
                look.on = switched_on(&before, &look.looks, look.on);
                for action in actions {
                    if matches!(action, Action::ImportLut) {
                        state.importing = true;
                    }
                    out.push(action);
                }
            });
    });
}

/// Whether the world look is on once the stack went from `before` to
/// `after`: a look chosen, moved, or set while it is off switches it on,
/// so that the change shows; "No filter" leaves the switch as it is.
pub fn switched_on(before: &[Layer], after: &[Layer], on: bool) -> bool {
    on || (after != before && !after.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_of_the_stack_switches_the_world_look_on() {
        let noir = vec![Layer::new("pack:czar-fenris/Noir")];
        assert!(switched_on(&[], &noir, false), "a look chosen while off");
        let mut weaker = noir.clone();
        weaker[0].strength = 40;
        assert!(switched_on(&noir, &weaker, false), "a look's strength changed");
        assert!(!switched_on(&noir, &noir, false), "nothing changed: stays off");
        assert!(!switched_on(&noir, &[], false), "\"No filter\": stays off");
        assert!(switched_on(&noir, &[], true), "\"No filter\" does not switch it off");
    }
}
