//! Parcel abilities of the navigation bar: voice, fly, push, build, scripts,
//! damage and avatar visibility, one icon each at the right end of the
//! location field; a frozen avatar (ViewerFrozenMessage) gets one more in
//! front of them while it lasts.
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
    /// Tabler icon name: filled where Tabler has one (the small shapes read
    /// better in green / red on the bar), outline otherwise.
    pub icon: &'static str,
    /// Green when true, red when false.
    pub ok: bool,
    /// Struck through when red (forbidden), unless the icon has its own slash
    /// or red means "active" (damage).
    pub strike: bool,
    /// Short text drawn after the icon (health when damage is on).
    pub label: Option<String>,
    pub tip: String,
    /// The ability in a word or two, written next to the icon by the place
    /// island of the Aurora bars when there is room for it.
    pub name: &'static str,
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
    /// Frozen by a parcel owner or an estate manager (ViewerFrozenMessage).
    pub frozen: bool,
}

/// allowAgentBuild / LLParcel::allowModifyBy, with "rez under land group":
/// the parcel is ours or no one's, open to all, or to its group we are in.
pub fn agent_can_build(s: &ParcelState) -> bool {
    let parcel = |f: u32| s.parcel.flags & f != 0;
    let owner = s.parcel.owner_id.is_nil() || (!s.parcel.is_group_owned && s.parcel.owner_id == s.agent_id);
    let group_create = s
        .group
        .is_some_and(|g| parcel(pf::CREATE_GROUP_OBJECTS) || g.powers & GP_LAND_ALLOW_CREATE != 0);
    owner || parcel(pf::CREATE_OBJECTS) || group_create
}

/// The icons, in Firestorm's order (voice, fly, push, build, scripts,
/// damage, see avatars), after the frozen state when there is one.
pub fn parcel_icons(s: &ParcelState) -> Vec<ParcelIcon> {
    let flags = s.parcel.flags;
    let region = |f: u32| s.region_flags & f != 0;
    let parcel = |f: u32| flags & f != 0;
    let mut out = Vec::with_capacity(8);

    // process_frozen_message shows nothing; Aurora keeps the state in sight
    // until the simulator releases us. Red without a strike: it is active.
    if s.frozen {
        let mut f = icon(
            "snowflake",
            false,
            aurora_i18n::tr!("ui-parcel-icons-name-frozen"),
            aurora_i18n::tr!("ui-parcel-icons-frozen-you-cannot-move-or-interact"),
        );
        f.strike = false;
        out.push(f);
    }

    // allowAgentVoice: region voice and parcel voice
    let voice = region(rf::ALLOW_VOICE) && parcel(pf::ALLOW_VOICE_CHAT);
    out.push(icon(
        "microphone-filled",
        voice,
        if voice {
            aurora_i18n::tr!("ui-parcel-icons-name-voice")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-no-voice")
        },
        if voice {
            aurora_i18n::tr!("ui-parcel-icons-voice-allowed")
        } else if !region(rf::ALLOW_VOICE) {
            aurora_i18n::tr!("ui-parcel-icons-voice-disabled-in-this-region-you-will-not-hear")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-voice-disabled-on-this-parcel-you-will-not-hear")
        },
    ));

    // allowAgentFly
    let fly = !region(rf::BLOCK_FLY) && parcel(pf::ALLOW_FLY);
    out.push(icon(
        "plane-tilt-filled",
        fly,
        if fly {
            aurora_i18n::tr!("ui-parcel-icons-name-fly")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-no-fly")
        },
        if fly {
            aurora_i18n::tr!("ui-parcel-icons-flight-allowed")
        } else if region(rf::BLOCK_FLY) {
            aurora_i18n::tr!("ui-parcel-icons-flight-disabled-in-this-region-you-cannot-fly-here")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-flight-disabled-on-this-parcel-you-cannot-fly-here")
        },
    ));

    // allowAgentPush: pushes by others' scripts (llPushObject)
    let push = !region(rf::RESTRICT_PUSHOBJECT) && !parcel(pf::RESTRICT_PUSHOBJECT);
    out.push(icon(
        "hand-stop",
        push,
        if push {
            aurora_i18n::tr!("ui-parcel-icons-name-push")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-no-push")
        },
        if push {
            aurora_i18n::tr!("ui-parcel-icons-pushing-allowed-other-people-s-scripts-can-push-you")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-pushing-restricted-only-the-landowner-can-push-others")
        },
    ));

    let build = agent_can_build(s);
    out.push(icon(
        "box",
        build,
        if build {
            aurora_i18n::tr!("ui-parcel-icons-name-build")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-no-build")
        },
        if build && parcel(pf::CREATE_OBJECTS) {
            aurora_i18n::tr!("ui-parcel-icons-building-allowed-for-everyone-you-can-create-and-rez")
        } else if build {
            aurora_i18n::tr!("ui-parcel-icons-building-allowed-for-you-parcel-owner-or-group")
        } else if parcel(pf::CREATE_GROUP_OBJECTS) {
            aurora_i18n::tr!("ui-parcel-icons-building-restricted-to-the-parcel-group-you-cannot-create")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-building-prohibited-you-cannot-create-or-rez-objects-here")
        },
    ));

    // allowAgentScripts (scripts of visitors' objects); the reasons of
    // onParcelIconClick
    let scripts = !region(rf::SKIP_SCRIPTS) && !region(rf::ESTATE_SKIP_SCRIPTS) && parcel(pf::ALLOW_OTHER_SCRIPTS);
    out.push(icon(
        "code",
        scripts,
        if scripts {
            aurora_i18n::tr!("ui-parcel-icons-name-scripts")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-no-scripts")
        },
        if scripts {
            aurora_i18n::tr!("ui-parcel-icons-scripts-allowed")
        } else if region(rf::ESTATE_SKIP_SCRIPTS) {
            aurora_i18n::tr!("ui-parcel-icons-an-administrator-has-temporarily-stopped-scripts-in-this-region")
        } else if region(rf::SKIP_SCRIPTS) {
            aurora_i18n::tr!("ui-parcel-icons-no-scripts-are-running-in-this-region")
        } else if parcel(pf::ALLOW_GROUP_SCRIPTS) {
            aurora_i18n::tr!("ui-parcel-icons-visitor-scripts-disabled-only-the-land-owner-s-and")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-visitor-scripts-disabled-only-the-land-owner-s-scripts")
        },
    ));

    // allowAgentDamage: green while safe, red with the health when damage is on
    let damage = region(rf::ALLOW_DAMAGE) || parcel(pf::ALLOW_DAMAGE);
    let health = s.health.clamp(0.0, 100.0) as i32;
    let mut d = icon(
        "heart-filled",
        !damage,
        if damage {
            aurora_i18n::tr!("ui-parcel-icons-name-damage")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-safe")
        },
        &if damage {
            aurora_i18n::tr!(
                "ui-parcel-icons-damage-allowed-you-can-be-hurt-here-health-if",
                health = format!("{}", health)
            )
        } else {
            aurora_i18n::tr!("ui-parcel-icons-safe-zone-no-damage").to_owned()
        },
    );
    d.label = damage.then(|| format!("{health}%"));
    d.strike = false;
    out.push(d);

    // LLParcel::getSeeAVs
    let see = s.parcel.see_avatars;
    out.push(icon(
        if see { "eye-filled" } else { "eye-off" },
        see,
        if see {
            aurora_i18n::tr!("ui-parcel-icons-name-see")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-name-isolated")
        },
        if see {
            aurora_i18n::tr!("ui-parcel-icons-avatars-visible-people-on-other-parcels-can-see-and")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-isolated-parcel-you-cannot-see-residents-on-other-parcels")
        },
    ));
    out
}

fn icon(icon: &'static str, ok: bool, name: &'static str, tip: &str) -> ParcelIcon {
    ParcelIcon {
        icon,
        ok,
        strike: !ok && !icon.ends_with("-off"),
        label: None,
        tip: tip.to_owned(),
        name,
    }
}

/// The tooltip's first clause (before " : " / ": "), for a row of the place
/// menu: "Flight disabled on this parcel" of "Flight disabled on this
/// parcel: you cannot fly here.".
pub fn headline(tip: &str) -> &str {
    let cut = [" : ", ": "].iter().filter_map(|s| tip.find(s)).min().unwrap_or(tip.len());
    tip[..cut].trim_end_matches('.')
}

const GAP: f32 = 2.0;
/// Height of an icon's slot in the location field, the side of its glyph
/// and the width of a slot without a label: the landmark star of the bar
/// (`bars.rs`) uses the same ones to line up with these icons.
pub const SLOT_HEIGHT: f32 = 18.0;
pub const GLYPH: f32 = 14.0;
pub const SLOT_WIDTH: f32 = 16.0;

/// The glyph's square in a slot: centered on the slot's height, half a slot
/// from its left edge.
pub fn glyph_rect(slot: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(egui::pos2(slot.left() + SLOT_WIDTH / 2.0, slot.center().y), Vec2::splat(GLYPH))
}

/// Tabler-style slash over a glyph: a gap in the colour `under` it, then
/// the line in `col`.
pub fn paint_strike(painter: &egui::Painter, glyph: egui::Rect, under: Color32, col: Color32) {
    let a = glyph.left_top() + Vec2::splat(0.5);
    let b = glyph.right_bottom() - Vec2::splat(0.5);
    painter.line_segment([a, b], egui::Stroke::new(3.2, under));
    painter.line_segment([a, b], egui::Stroke::new(1.3, col));
}

fn label_font() -> egui::FontId {
    egui::FontId::proportional(10.5)
}

fn item_width(ui: &egui::Ui, it: &ParcelIcon) -> f32 {
    SLOT_WIDTH
        + it.label.as_ref().map_or(0.0, |t| {
            ui.painter().layout_no_wrap(t.clone(), label_font(), Color32::PLACEHOLDER).size().x + 2.0
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
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, SLOT_HEIGHT), egui::Sense::click());
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect.expand(1.0), ui.visuals().window_corner_radius, p.raised);
        }
        let ir = glyph_rect(rect);
        if let Some(t) = icons.get(it.icon) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(t.id(), ir, uv, col);
        }
        if it.strike {
            paint_strike(ui.painter(), ir, if resp.hovered() { p.raised } else { bg }, col);
        }
        if let Some(g) = label {
            let pos = crate::ui::widgets::row_text_origin(ui.painter(), &g, ir.right() + 2.0, rect.center().y);
            ui.painter().galley(pos, g, col);
        }
        let resp = resp.on_hover_text(aurora_i18n::tr!("ui-parcel-icons-click-about-land", value0 = it.tip.to_string()));
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
            frozen: false,
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
                ("microphone-filled", false),
                ("plane-tilt-filled", false),
                ("hand-stop", false),
                ("box", false),
                ("code", false),
                ("heart-filled", false),
                ("eye-filled", true),
            ]
        );
        assert!(icons[4].tip == aurora_i18n::tr!("ui-parcel-icons-an-administrator-has-temporarily-stopped-scripts-in-this-region"));
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

    /// The place menu writes the first clause of a tooltip, in both languages'
    /// punctuation; each right has a short name for the place island.
    #[test]
    fn a_right_reads_in_a_few_words() {
        assert_eq!(
            headline("Vol interdit sur cette parcelle : vous ne pouvez pas voler ici."),
            "Vol interdit sur cette parcelle"
        );
        assert_eq!(
            headline("Flight disabled on this parcel: you cannot fly here."),
            "Flight disabled on this parcel"
        );
        assert_eq!(headline("Scripts allowed"), "Scripts allowed");
        assert_eq!(
            headline("Aucun script ne fonctionne dans cette région."),
            "Aucun script ne fonctionne dans cette région"
        );
        let parcel = ParcelInfo::default();
        for it in parcel_icons(&state(0, &parcel)) {
            assert!(!it.name.is_empty() && it.name.len() < it.tip.len(), "{}", it.icon);
        }
    }

    #[test]
    fn a_frozen_avatar_sees_it_first() {
        let parcel = ParcelInfo::default();
        assert!(parcel_icons(&state(0, &parcel)).iter().all(|i| i.icon != "snowflake"));
        let s = ParcelState {
            frozen: true,
            ..state(0, &parcel)
        };
        let icons = parcel_icons(&s);
        assert_eq!(icons.len(), 8);
        assert_eq!((icons[0].icon, icons[0].ok, icons[0].strike), ("snowflake", false, false));
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
        assert_eq!((icons[6].icon, icons[6].ok, icons[6].strike), ("eye-off", false, false));
    }
}
