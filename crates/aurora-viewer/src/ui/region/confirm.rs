//! The questions and notices of the Region / Estate window, as Firestorm
//! raises them (notifications.xml): EstateParcelAccessOverride when parcels
//! may no longer be more restrictive, ChangeLindenEstate before changing a
//! Linden estate, ChangeObjectBonusFactor before a new object bonus (its
//! "ignore" box), RegionMaturityChange once a new rating is sent,
//! ConfirmTextureHeights for inverted elevation ranges (its "Don't ask"
//! turns RegionCheckTextureHeights off), ConfirmBakeTerrain,
//! SettingsConfirmReset before the region goes back to the default
//! environment, EstateParcelEnvironmentOverride (ChangeLindenEstate on a
//! Linden estate) for the parcels' own environments and
//! EstateChangeCovenant before the covenant is replaced or cleared. One is
//! Aurora's own: an access change that shuts people out (the lists only, a
//! public limit turned on) is confirmed before it is sent, with how many
//! people are in the region.

use super::advanced::MaterialProblem;
use super::{RegionUi, View, estate_wide, manage};
use crate::theme::Palette;
use crate::ui::sheet::semibold;
use crate::ui::widgets::{self, ActionKind, DialogButton};
use crate::world::World;
use egui::{RichText, Sense, Vec2};

/// What the window asks or tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    AccessOverride,
    LindenEstate,
    /// The access shuts people out: `Access::restrictions`' flags, and
    /// how many other people the region's coarse locations count.
    RestrictAccess {
        what: u64,
        here: usize,
    },
    ObjectBonus {
        dont_ask: bool,
    },
    MaturityChanged,
    TextureHeights,
    Bake,
    ResetEnvironment,
    EnvironmentOverride {
        allow: bool,
    },
    /// A notecard's asset, or nil for "Reset".
    Covenant {
        asset: uuid::Uuid,
    },
    /// InvalidTerrainMaterial…: a terrain slot's material cannot be used.
    TerrainMaterial {
        slot: u8,
        problem: MaterialProblem,
    },
}

/// The answer's buttons, the last one the main action.
enum Choice {
    Ok,
    Cancel,
    DontAsk,
}

/// Show the question asked, if any; true when a preference changed.
pub(super) fn show(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) -> bool {
    let Some(mut ask) = s.ask.take() else {
        return false;
    };
    let ok = aurora_i18n::tr!("common-ok");
    let cancel = aurora_i18n::tr!("common-cancel");
    let dont_ask = aurora_i18n::tr!("region-ask-dont-ask");
    let linden = world
        .estate
        .estate
        .as_ref()
        .is_some_and(|e| e.id <= aurora_net::estate::ESTATE_LAST_LINDEN);
    let estate_text = aurora_i18n::tr!(
        "region-ask-env-override",
        estate = world.estate.estate.as_ref().map(|e| e.name.clone()).unwrap_or_default()
    );
    let material_text = match ask {
        Ask::TerrainMaterial { slot, problem } => {
            let n = i64::from(slot) + 1;
            match problem {
                MaterialProblem::NotLoaded => aurora_i18n::tr!("region-ask-material-not-loaded", n = n),
                MaterialProblem::LoadFailed => aurora_i18n::tr!("region-ask-material-failed", n = n),
                MaterialProblem::DoubleSided => aurora_i18n::tr!("region-ask-material-double-sided", n = n),
                MaterialProblem::Blend => aurora_i18n::tr!("region-ask-material-blend", n = n),
            }
        }
        _ => String::new(),
    };
    let restrict_text = match ask {
        Ask::RestrictAccess { what, here } => restrict_text(what, here),
        _ => String::new(),
    };
    let (icon, danger, title, text) = match ask {
        Ask::AccessOverride => (
            "alert-triangle-filled",
            false,
            aurora_i18n::tr!("region-ask-override-title"),
            aurora_i18n::tr!("region-ask-override"),
        ),
        Ask::LindenEstate => (
            "alert-triangle-filled",
            true,
            aurora_i18n::tr!("region-ask-linden-title"),
            aurora_i18n::tr!("region-ask-linden"),
        ),
        Ask::RestrictAccess { .. } => (
            "alert-triangle-filled",
            true,
            aurora_i18n::tr!("region-ask-restrict-title"),
            &*restrict_text,
        ),
        Ask::ObjectBonus { .. } => (
            "alert-triangle-filled",
            false,
            aurora_i18n::tr!("region-ask-bonus-title"),
            aurora_i18n::tr!("region-ask-bonus"),
        ),
        Ask::MaturityChanged => (
            "info-circle-filled",
            false,
            aurora_i18n::tr!("region-maturity-changed-title"),
            aurora_i18n::tr!("region-maturity-changed"),
        ),
        Ask::TextureHeights => (
            "alert-triangle-filled",
            false,
            aurora_i18n::tr!("region-ask-heights-title"),
            aurora_i18n::tr!("region-ask-heights"),
        ),
        Ask::Bake => (
            "mountain-filled",
            false,
            aurora_i18n::tr!("region-ask-bake-title"),
            aurora_i18n::tr!("region-ask-bake"),
        ),
        Ask::ResetEnvironment => (
            "sun-high-filled",
            false,
            aurora_i18n::tr!("region-ask-env-reset-title"),
            aurora_i18n::tr!("region-ask-env-reset"),
        ),
        Ask::EnvironmentOverride { .. } if linden => (
            "alert-triangle-filled",
            true,
            aurora_i18n::tr!("region-ask-linden-title"),
            aurora_i18n::tr!("region-ask-linden"),
        ),
        Ask::EnvironmentOverride { .. } => (
            "alert-triangle-filled",
            false,
            aurora_i18n::tr!("region-ask-env-override-title"),
            &*estate_text,
        ),
        Ask::TerrainMaterial { .. } => (
            "alert-triangle-filled",
            false,
            aurora_i18n::tr!("region-ask-material-title"),
            &*material_text,
        ),
        Ask::Covenant { asset } => (
            "gavel",
            asset.is_nil(),
            aurora_i18n::tr!("region-ask-covenant-title"),
            if asset.is_nil() {
                aurora_i18n::tr!("region-ask-covenant-reset")
            } else {
                aurora_i18n::tr!("region-ask-covenant")
            },
        ),
    };
    // the main button says what happens, as the mockup's confirmations do
    let (main, main_kind) = match ask {
        Ask::LindenEstate => (aurora_i18n::tr!("region-ask-linden-ok"), ActionKind::Danger),
        Ask::RestrictAccess { .. } => (aurora_i18n::tr!("region-ask-restrict-ok"), ActionKind::Danger),
        Ask::ObjectBonus { .. } => (aurora_i18n::tr!("region-ask-bonus-ok"), ActionKind::Primary),
        Ask::TextureHeights => (aurora_i18n::tr!("region-ask-heights-ok"), ActionKind::Primary),
        Ask::Bake => (aurora_i18n::tr!("region-ask-bake-ok"), ActionKind::Primary),
        Ask::ResetEnvironment => (aurora_i18n::tr!("region-ask-env-reset-ok"), ActionKind::Primary),
        Ask::EnvironmentOverride { .. } if linden => (aurora_i18n::tr!("region-ask-linden-ok"), ActionKind::Danger),
        Ask::EnvironmentOverride { .. } => (aurora_i18n::tr!("region-ask-env-override-ok"), ActionKind::Primary),
        Ask::Covenant { asset } if asset.is_nil() => (aurora_i18n::tr!("region-ask-covenant-reset-ok"), ActionKind::Danger),
        Ask::Covenant { .. } => (aurora_i18n::tr!("region-ask-covenant-ok"), ActionKind::Primary),
        Ask::AccessOverride | Ask::MaturityChanged | Ask::TerrainMaterial { .. } => (ok, ActionKind::Primary),
    };
    // a notice has its OK only
    let notice = matches!(ask, Ask::AccessOverride | Ask::MaturityChanged | Ask::TerrainMaterial { .. });
    let mut buttons: Vec<(DialogButton<'_>, Choice)> = Vec::new();
    if matches!(ask, Ask::TextureHeights) {
        buttons.push((button(dont_ask, ActionKind::Flat), Choice::DontAsk));
    }
    if !notice {
        buttons.push((button(cancel, ActionKind::Flat), Choice::Cancel));
    }
    buttons.push((button(main, main_kind), Choice::Ok));
    let mut answer = None;
    let modal = widgets::Modal::new(egui::Id::unique("region_estate_ask")).show(ctx, |ui| {
        // wider when its buttons, all of the widest one's width, need it
        let widest = buttons
            .iter()
            .map(|(b, _)| widgets::action_button_fit(ui, b.icon, b.label))
            .fold(0.0_f32, f32::max);
        let count = buttons.len() as f32;
        ui.set_width(380.0_f32.max(count * widest + (count - 1.0) * ui.spacing().item_spacing.x));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let tint = if danger { p.danger } else { p.violet_light };
            widgets::glyph(ui, icon, tint, 16.0);
            ui.label(semibold(ui, title, 13.5, p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, r) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                crate::ui::sheet::icon_at(ui, "x", rect.center(), 15.0, if r.hovered() { p.ink } else { p.muted });
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    answer = Some(Choice::Cancel);
                }
            });
        });
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
        if let Ask::ObjectBonus { dont_ask: ref mut off } = ask {
            ui.add_space(8.0);
            widgets::checkbox(ui, p, off, aurora_i18n::tr!("region-ask-dont-ask"));
        }
        ui.add_space(14.0);
        let list: Vec<DialogButton<'_>> = buttons
            .iter()
            .map(|(b, _)| DialogButton {
                icon: b.icon,
                label: b.label,
                kind: b.kind,
                enabled: b.enabled,
            })
            .collect();
        if let Some(i) = widgets::dialog_buttons(ui, p, &list, 28.0) {
            answer = Some(match buttons[i].1 {
                Choice::Ok => Choice::Ok,
                Choice::Cancel => Choice::Cancel,
                Choice::DontAsk => Choice::DontAsk,
            });
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(if notice { Choice::Ok } else { Choice::Cancel });
    }
    let Some(answer) = answer else {
        s.ask = Some(ask);
        return false;
    };
    let mut prefs = false;
    match (ask, answer) {
        (Ask::RestrictAccess { .. }, Choice::Ok) => manage::access_confirmed(s, v, world),
        (Ask::LindenEstate, Choice::Ok) => manage::send_access(s, v, world),
        (Ask::ObjectBonus { dont_ask }, Choice::Ok) => {
            if dont_ask {
                s.confirm_object_bonus = false;
                prefs = true;
            }
            manage::send_rules(s, v, world);
        }
        (Ask::TextureHeights, Choice::DontAsk) => {
            s.check_texture_heights = false;
            prefs = true;
            manage::send_terrain(s, v, world);
        }
        (Ask::TextureHeights, Choice::Ok) => manage::send_terrain(s, v, world),
        (Ask::Bake, Choice::Ok) => world.estate.bake_terrain(v.handle),
        (Ask::ResetEnvironment, Choice::Ok) => estate_wide::reset_environment(world),
        (Ask::EnvironmentOverride { allow }, Choice::Ok) => estate_wide::set_override(v, world, allow),
        (Ask::Covenant { asset }, Choice::Ok) => estate_wide::change_covenant(v, world, asset),
        _ => {}
    }
    prefs
}

/// What the restriction does, one line per thing it shuts out, then who it
/// reaches and how many are here.
fn restrict_text(what: u64, here: usize) -> String {
    use aurora_net::estate::estate_flags as ef;
    let lines: Vec<&str> = [
        (ef::EXTERNALLY_VISIBLE, aurora_i18n::tr!("region-ask-restrict-lists")),
        (ef::DENY_AGEUNVERIFIED, aurora_i18n::tr!("region-ask-restrict-age")),
        (ef::DENY_ANONYMOUS, aurora_i18n::tr!("region-ask-restrict-payment")),
        (ef::DENY_BOTS, aurora_i18n::tr!("region-ask-restrict-bots")),
    ]
    .into_iter()
    .filter(|(flag, _)| what & flag != 0)
    .map(|(_, line)| line)
    .collect();
    let mut text = lines.join("\n");
    text.push_str("\n\n");
    text.push_str(aurora_i18n::tr!("region-ask-restrict-sent-home"));
    if here > 0 {
        text.push_str("\n\n");
        text.push_str(&aurora_i18n::tr!("region-ask-restrict-here", count = here as i64));
    }
    text
}

fn button(label: &str, kind: ActionKind) -> DialogButton<'_> {
    DialogButton {
        icon: "",
        label,
        kind,
        enabled: true,
    }
}
