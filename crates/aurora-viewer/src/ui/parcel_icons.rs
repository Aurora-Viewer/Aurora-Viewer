//! Parcel abilities of the navigation bar: voice, fly, push, build, scripts,
//! damage and avatar visibility, one icon each at the right end of the
//! location field.
//!
//! Rules ported from LLViewerParcelMgr::allowAgentVoice / allowAgentFly /
//! allowAgentPush / allowAgentBuild / allowAgentScripts / allowAgentDamage and
//! LLLocationInputCtrl::refreshParcelIcons / onParcelIconClick
//! (indra/newview/llviewerparcelmgr.cpp, lllocationinputctrl.cpp, originally
//! LGPL 2.1).
//!
//! Deliberate differences from Firestorm, which only shows the restrictions
//! (and damage) as icons: every ability is shown, green when allowed, red and
//! struck through when not, so the parcel rules read at a glance. Build is the
//! agent's own right (owner, group, `GP_LAND_ALLOW_CREATE`, like Firestorm's
//! "rez under land group") rather than "anyone can build" only. A click opens
//! About Land instead of a notification (the tooltip already explains).

use super::icons::Icons;
use crate::theme::Palette;
use aurora_net::{GroupMembership, ParcelInfo, parcel_flags as pf, region_flags as rf};
use egui::{Color32, Vec2};
use uuid::Uuid;

/// GP_LAND_ALLOW_CREATE (roles_constants.h): bypass the parcel's
/// create / edit objects restriction.
const GP_LAND_ALLOW_CREATE: u64 = 1 << 25;

/// One ability of the current parcel.
#[derive(Debug, Clone, PartialEq)]
pub struct ParcelIcon {
    /// Phosphor icon name, "fill" weight (chosen by the user: the small
    /// shapes read better in green / red on the bar than the regular outline).
    pub icon: &'static str,
    /// Green when true, red when false.
    pub ok: bool,
    /// Struck through when red (forbidden), unless the icon has its own slash
    /// or red means "active" (damage).
    pub strike: bool,
    /// Short text drawn after the icon (health when damage is on).
    pub label: Option<String>,
    pub tip: String,
}

/// What the rules need to know about where the agent stands.
pub struct ParcelState<'a> {
    pub region_flags: u32,
    pub parcel: &'a ParcelInfo,
    pub agent_id: Uuid,
    /// The agent's membership in the parcel's group, if any.
    pub group: Option<&'a GroupMembership>,
    /// Health in percent (HealthMessage).
    pub health: f32,
}

/// The icons, in Firestorm's order (voice, fly, push, build, scripts,
/// damage, see avatars).
pub fn parcel_icons(s: &ParcelState) -> Vec<ParcelIcon> {
    let flags = s.parcel.flags;
    let region = |f: u32| s.region_flags & f != 0;
    let parcel = |f: u32| flags & f != 0;
    let mut out = Vec::with_capacity(7);

    // allowAgentVoice: region voice and parcel voice
    let voice = region(rf::ALLOW_VOICE) && parcel(pf::ALLOW_VOICE_CHAT);
    out.push(icon(
        "microphone-fill",
        voice,
        if voice {
            "Voix autorisée"
        } else if !region(rf::ALLOW_VOICE) {
            "Voix interdite dans cette région : vous n'entendrez personne parler."
        } else {
            "Voix interdite sur cette parcelle : vous n'entendrez personne parler."
        },
    ));

    // allowAgentFly
    let fly = !region(rf::BLOCK_FLY) && parcel(pf::ALLOW_FLY);
    out.push(icon(
        "airplane-tilt-fill",
        fly,
        if fly {
            "Vol autorisé"
        } else if region(rf::BLOCK_FLY) {
            "Vol interdit dans cette région : vous ne pouvez pas voler ici."
        } else {
            "Vol interdit sur cette parcelle : vous ne pouvez pas voler ici."
        },
    ));

    // allowAgentPush: pushes by others' scripts (llPushObject)
    let push = !region(rf::RESTRICT_PUSHOBJECT) && !parcel(pf::RESTRICT_PUSHOBJECT);
    out.push(icon(
        "hand-palm-fill",
        push,
        if push {
            "Bousculades autorisées : les scripts des autres peuvent vous pousser."
        } else {
            "Bousculades interdites : personne ne peut pousser les autres, sauf le propriétaire du terrain."
        },
    ));

    // allowAgentBuild / LLParcel::allowModifyBy, with FS rez under land group
    let owner = s.parcel.owner_id.is_nil() || (!s.parcel.is_group_owned && s.parcel.owner_id == s.agent_id);
    let group_create = s
        .group
        .is_some_and(|g| parcel(pf::CREATE_GROUP_OBJECTS) || g.powers & GP_LAND_ALLOW_CREATE != 0);
    let build = owner || parcel(pf::CREATE_OBJECTS) || group_create;
    out.push(icon(
        "cube-fill",
        build,
        if build && parcel(pf::CREATE_OBJECTS) {
            "Construction autorisée à tous : vous pouvez créer et poser des objets."
        } else if build {
            "Construction autorisée pour vous (propriétaire ou groupe de la parcelle)."
        } else if parcel(pf::CREATE_GROUP_OBJECTS) {
            "Construction réservée au groupe de la parcelle : vous ne pouvez pas créer ni poser d'objets ici."
        } else {
            "Construction interdite : vous ne pouvez pas créer ni poser d'objets ici."
        },
    ));

    // allowAgentScripts (scripts of visitors' objects); the reasons of
    // onParcelIconClick
    let scripts = !region(rf::SKIP_SCRIPTS) && !region(rf::ESTATE_SKIP_SCRIPTS) && parcel(pf::ALLOW_OTHER_SCRIPTS);
    out.push(icon(
        "code-fill",
        scripts,
        if scripts {
            "Scripts autorisés"
        } else if region(rf::ESTATE_SKIP_SCRIPTS) {
            "Un administrateur a temporairement arrêté les scripts dans cette région."
        } else if region(rf::SKIP_SCRIPTS) {
            "Aucun script ne fonctionne dans cette région."
        } else if parcel(pf::ALLOW_GROUP_SCRIPTS) {
            "Scripts des visiteurs désactivés : seuls ceux du propriétaire et du groupe du terrain fonctionnent."
        } else {
            "Scripts des visiteurs désactivés : seuls ceux du propriétaire du terrain fonctionnent."
        },
    ));

    // allowAgentDamage: green while safe, red with the health when damage is on
    let damage = region(rf::ALLOW_DAMAGE) || parcel(pf::ALLOW_DAMAGE);
    let health = s.health.clamp(0.0, 100.0) as i32;
    let mut d = icon(
        "heart-fill",
        !damage,
        &if damage {
            format!(
                "Dégâts autorisés : vous pouvez être blessé ici (santé {health} %). \
                 Si vous mourez, vous serez téléporté chez vous."
            )
        } else {
            "Zone sûre : pas de dégâts".to_owned()
        },
    );
    d.label = damage.then(|| format!("{health}%"));
    d.strike = false;
    out.push(d);

    // LLParcel::getSeeAVs
    let see = s.parcel.see_avatars;
    out.push(icon(
        if see { "eye-fill" } else { "eye-slash-fill" },
        see,
        if see {
            "Avatars visibles : on vous voit et vous entend depuis les autres parcelles."
        } else {
            "Parcelle isolée : vous ne voyez pas les résidents des autres parcelles, ils ne vous voient pas \
             et le chat local ne passe pas."
        },
    ));
    out
}

fn icon(icon: &'static str, ok: bool, tip: &str) -> ParcelIcon {
    ParcelIcon {
        icon,
        ok,
        strike: !ok && !icon.contains("-slash"),
        label: None,
        tip: tip.to_owned(),
    }
}

const GAP: f32 = 2.0;

fn label_font() -> egui::FontId {
    egui::FontId::proportional(10.5)
}

fn item_width(ui: &egui::Ui, it: &ParcelIcon) -> f32 {
    16.0 + it.label.as_ref().map_or(0.0, |t| {
        ui.painter().layout_no_wrap(t.clone(), label_font(), Color32::WHITE).size().x + 2.0
    })
}

/// Width taken by `show` (to reserve it at the right of the field).
pub fn width(ui: &egui::Ui, items: &[ParcelIcon]) -> f32 {
    let n = items.len() as f32;
    items.iter().map(|it| item_width(ui, it)).sum::<f32>() + GAP * (n - 1.0).max(0.0)
}

/// Draw the icons left to right; `bg` is the color under them (for the gap
/// around the strike line). Returns true when one is clicked.
pub fn show(ui: &mut egui::Ui, p: &Palette, icons: &Icons, items: &[ParcelIcon], bg: Color32) -> bool {
    let mut clicked = false;
    ui.spacing_mut().item_spacing.x = GAP;
    for it in items {
        let col = if it.ok { p.success } else { p.danger };
        let label = it.label.as_ref().map(|t| ui.painter().layout_no_wrap(t.clone(), label_font(), col));
        let w = item_width(ui, it);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 18.0), egui::Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect.expand(1.0), 2.0, p.raised);
        }
        let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 8.0, rect.center().y), Vec2::splat(14.0));
        if let Some(t) = icons.get(it.icon) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(t.id(), ir, uv, col);
        }
        if it.strike {
            // Phosphor-style slash: a gap in the background color, then the line
            let a = ir.left_top() + Vec2::splat(0.5);
            let b = ir.right_bottom() - Vec2::splat(0.5);
            let under = if resp.hovered() { p.raised } else { bg };
            ui.painter().line_segment([a, b], egui::Stroke::new(3.2, under));
            ui.painter().line_segment([a, b], egui::Stroke::new(1.3, col));
        }
        if let Some(g) = label {
            let pos = egui::pos2(ir.right() + 2.0, rect.center().y - g.size().y / 2.0);
            ui.painter().galley(pos, g, col);
        }
        let resp = resp.on_hover_text(format!("{}\nClic : à propos du terrain", it.tip));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        clicked |= resp.clicked();
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(region_flags: u32, parcel: &ParcelInfo) -> ParcelState<'_> {
        ParcelState {
            region_flags,
            parcel,
            agent_id: Uuid::from_u128(1),
            group: None,
            health: 100.0,
        }
    }

    fn oks(icons: &[ParcelIcon]) -> Vec<(&'static str, bool)> {
        icons.iter().map(|i| (i.icon, i.ok)).collect()
    }

    #[test]
    fn open_parcel_is_all_green() {
        let parcel = ParcelInfo {
            owner_id: Uuid::from_u128(2),
            flags: pf::ALLOW_VOICE_CHAT | pf::ALLOW_FLY | pf::CREATE_OBJECTS | pf::ALLOW_OTHER_SCRIPTS,
            see_avatars: true,
            ..Default::default()
        };
        let icons = parcel_icons(&state(rf::ALLOW_VOICE, &parcel));
        assert!(icons.iter().all(|i| i.ok), "{:?}", oks(&icons));
        assert_eq!(icons.len(), 7);
        assert!(icons[5].label.is_none());
    }

    #[test]
    fn region_flags_override_the_parcel() {
        let parcel = ParcelInfo {
            owner_id: Uuid::from_u128(2),
            flags: pf::ALLOW_VOICE_CHAT | pf::ALLOW_FLY | pf::ALLOW_OTHER_SCRIPTS,
            see_avatars: true,
            ..Default::default()
        };
        // voice off for the region, fly blocked, push restricted, scripts stopped
        let icons = parcel_icons(&state(
            rf::BLOCK_FLY | rf::RESTRICT_PUSHOBJECT | rf::ESTATE_SKIP_SCRIPTS | rf::ALLOW_DAMAGE,
            &parcel,
        ));
        assert_eq!(
            oks(&icons),
            vec![
                ("microphone-fill", false),
                ("airplane-tilt-fill", false),
                ("hand-palm-fill", false),
                ("cube-fill", false),
                ("code-fill", false),
                ("heart-fill", false),
                ("eye-fill", true),
            ]
        );
        assert!(icons[4].tip.contains("administrateur"));
        assert_eq!(icons[5].label.as_deref(), Some("100%"));
    }

    #[test]
    fn build_rights_of_owner_and_group() {
        let group = Uuid::from_u128(9);
        let mut parcel = ParcelInfo {
            owner_id: Uuid::from_u128(1),
            group_id: group,
            ..Default::default()
        };
        // own parcel: always
        assert!(parcel_icons(&state(0, &parcel))[3].ok);
        // someone else's, group building on, not a member: no
        parcel.owner_id = Uuid::from_u128(2);
        parcel.flags = pf::CREATE_GROUP_OBJECTS;
        assert!(!parcel_icons(&state(0, &parcel))[3].ok);
        // member of the parcel group: yes
        let m = GroupMembership {
            id: group,
            name: String::new(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        };
        let s = ParcelState {
            group: Some(&m),
            ..state(0, &parcel)
        };
        assert!(parcel_icons(&s)[3].ok);
        // group building off, but the member may bypass the restriction
        parcel.flags = 0;
        let m2 = GroupMembership {
            powers: GP_LAND_ALLOW_CREATE,
            ..m.clone()
        };
        let s = ParcelState {
            group: Some(&m2),
            ..state(0, &parcel)
        };
        assert!(parcel_icons(&s)[3].ok);
        let s = ParcelState {
            group: Some(&m),
            ..state(0, &parcel)
        };
        assert!(!parcel_icons(&s)[3].ok);
    }

    #[test]
    fn hidden_avatars_and_health() {
        let parcel = ParcelInfo {
            flags: pf::ALLOW_DAMAGE,
            see_avatars: false,
            ..Default::default()
        };
        let s = ParcelState {
            health: 72.6,
            ..state(0, &parcel)
        };
        let icons = parcel_icons(&s);
        assert_eq!(icons[5].label.as_deref(), Some("72%"));
        assert!(!icons[5].strike);
        assert_eq!((icons[6].icon, icons[6].ok, icons[6].strike), ("eye-slash-fill", false, false));
    }
}
