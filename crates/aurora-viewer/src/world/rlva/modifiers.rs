//! What the RLVa tools list: the behaviours, their exceptions and the global
//! modifiers. Port of RlvFloaterBehaviours::refreshAll, rlvGetShowException
//! (indra/newview/rlvfloaters.cpp) and of the modifier table of
//! RlvBehaviourDictionary (indra/newview/rlvhelper.cpp), originally LGPL 2.1.
use super::{Rlva, Rule, canonical, im_range};
use crate::world::inventory::asset_ids::AssetIds;
use aurora_i18n::tr;
use uuid::Uuid;

/// rlvGetShowException: an option on one of these names someone the behaviour
/// makes an exception for. Any other option stays a restriction.
fn shows_as_exception(behaviour: &str) -> bool {
    matches!(
        behaviour,
        "recvchat"
            | "recvemote"
            | "sendim"
            | "recvim"
            | "startim"
            | "tplure"
            | "tprequest"
            | "accepttp"
            | "accepttprequest"
            | "shownames"
            | "shownametags"
    )
}

/// RLV_MODIFIER_FARTOUCH_DEFAULT, RLV_MODIFIER_SITTP_DEFAULT and
/// RLV_MODIFIER_TPLOCAL_DEFAULT (indra/newview/rlvdefines.h).
const DEFAULT_DISTANCE: f32 = 1.5;

fn listed_as_exception(rule: &Rule) -> bool {
    !rule.option.is_empty() && shows_as_exception(&rule.behaviour)
}

/// A row of the Exceptions tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exception {
    pub source: Uuid,
    pub behaviour: String,
    pub option: String,
    /// RlvHandler::isException for this id. A distance such as `recvim:20;50`
    /// lists here too, like the Legacy Viewer, and is never active.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModifierValue {
    Text(String),
    /// A texture a script names: written through the asset id rule.
    Asset(Uuid),
}

/// A row of the Modifiers tab.
#[derive(Debug, Clone, PartialEq)]
pub struct Modifier {
    pub name: &'static str,
    /// None while no behaviour sets it: the Legacy Viewer's "(default)".
    pub value: Option<ModifierValue>,
    /// The object whose value applies. The Legacy Viewer only names the
    /// exclusive camera owner there; naming every issuer tells more.
    pub source: Option<Uuid>,
}

fn number(value: Option<(f32, Uuid)>) -> (Option<ModifierValue>, Option<Uuid>) {
    match value {
        Some((value, source)) => (Some(ModifierValue::Text(format!("{value:.3}"))), Some(source)),
        None => (None, None),
    }
}

/// Which value of several applies (RlvBehaviourModifierCompMin / CompMax).
/// `largest` for CompMax, else the smallest.
pub(super) fn best(values: impl Iterator<Item = (f32, Uuid)>, largest: bool) -> Option<(f32, Uuid)> {
    values
        .filter(|(value, _)| !value.is_nan())
        .reduce(|a, b| if (largest && b.0 > a.0) || (!largest && b.0 < a.0) { b } else { a })
}

impl Rlva {
    /// The Restrictions tab: every rule but the listed exceptions.
    pub fn behaviours(&self) -> Vec<(Uuid, String)> {
        if !self.enabled {
            return Vec::new();
        }
        self.rules
            .iter()
            .flat_map(|(id, rules)| {
                rules
                    .iter()
                    .filter(|r| !listed_as_exception(r))
                    .map(move |r| (*id, super::rule_text(r)))
            })
            .collect()
    }

    /// The Exceptions tab.
    pub fn exceptions(&self) -> Vec<Exception> {
        if !self.enabled {
            return Vec::new();
        }
        self.rules
            .iter()
            .flat_map(|(id, rules)| rules.iter().filter(|r| listed_as_exception(r)).map(move |r| (*id, r)))
            .map(|(source, r)| Exception {
                source,
                behaviour: if r.strict {
                    format!("{}_sec", r.behaviour)
                } else {
                    r.behaviour.clone()
                },
                option: r.option.clone(),
                active: Uuid::parse_str(&r.option).is_ok_and(|id| !id.is_nil()) && self.is_exception(&r.behaviour, &r.option),
            })
            .collect()
    }

    /// `@bhvr[:<distance>]=n`: a behaviour with no option adds the default
    /// (RlvBehaviourGenericHandler<RLV_OPTION_NONE_OR_MODIFIER>). The closest
    /// wins, as `distance_limit` applies it. An option that is no number, the
    /// id of a `@shownametags` exception, sets nothing.
    fn distance(&self, name: &str, default: f32) -> Option<(f32, Uuid)> {
        best(
            self.rules_for(name, Uuid::nil()).filter_map(|(id, option)| {
                if option.is_empty() {
                    Some((default, id))
                } else {
                    option.parse().ok().map(|value| (value, id))
                }
            }),
            false,
        )
    }

    /// RlvBehaviourRecvSendStartIMHandler: `<min>[;<max>]`. The Legacy Viewer
    /// stores and lists the squares; this lists the distance the script gave.
    fn im_distance(&self, name: &str, maximum: bool) -> Option<(f32, Uuid)> {
        best(
            self.rules_for(name, Uuid::nil()).filter_map(|(id, option)| {
                let (min, max) = im_range(option)?;
                if !maximum {
                    Some((min, id))
                } else if option.split_once(';').is_some_and(|(_, text)| !text.is_empty()) {
                    Some((max, id))
                } else {
                    None
                }
            }),
            !maximum,
        )
    }

    fn camera_text(&self, name: &str) -> (Option<ModifierValue>, Option<Uuid>) {
        if let Some((source, value)) = self.camera_rule(name) {
            return (Some(ModifierValue::Text(value.to_owned())), Some(source));
        }
        (self.camera_value(name).map(|value| ModifierValue::Text(value.to_owned())), None)
    }

    /// The Modifiers tab: every global modifier, set or not, in the order of
    /// ERlvBehaviourModifier (RlvFloaterBehaviours::refreshAll).
    pub fn modifiers(&self) -> Vec<Modifier> {
        let row = |name, (value, source)| Modifier { name, value, source };
        let camera = |name, minimum| number(self.camera_winner(name, minimum));
        vec![
            row(tr!("rlva-modifier-fartouch"), number(self.distance("fartouch", DEFAULT_DISTANCE))),
            row(tr!("rlva-modifier-recvim-min"), number(self.im_distance("recvim", false))),
            row(tr!("rlva-modifier-recvim-max"), number(self.im_distance("recvim", true))),
            row(tr!("rlva-modifier-sendim-min"), number(self.im_distance("sendim", false))),
            row(tr!("rlva-modifier-sendim-max"), number(self.im_distance("sendim", true))),
            row(tr!("rlva-modifier-startim-min"), number(self.im_distance("startim", false))),
            row(tr!("rlva-modifier-startim-max"), number(self.im_distance("startim", true))),
            row(tr!("rlva-modifier-silhouette"), camera("setcam_avdist", true)),
            row(tr!("rlva-modifier-avatar-min"), camera("setcam_avdistmin", true)),
            row(tr!("rlva-modifier-avatar-max"), camera("setcam_avdistmax", false)),
            row(tr!("rlva-modifier-focus-min"), camera("setcam_origindistmin", true)),
            row(tr!("rlva-modifier-focus-max"), camera("setcam_origindistmax", false)),
            row(tr!("rlva-modifier-eye-offset"), self.camera_text("setcam_eyeoffset")),
            row(tr!("rlva-modifier-eye-scale"), self.camera_text("setcam_eyeoffsetscale")),
            row(tr!("rlva-modifier-focus-offset"), self.camera_text("setcam_focusoffset")),
            row(tr!("rlva-modifier-fov-min"), camera("setcam_fovmin", true)),
            row(tr!("rlva-modifier-fov-max"), camera("setcam_fovmax", false)),
            row(
                tr!("rlva-modifier-texture"),
                (
                    self.camera_texture().map(ModifierValue::Asset),
                    self.camera_rule("setcam_textures").map(|(source, _)| source),
                ),
            ),
            row(tr!("rlva-modifier-nametags"), number(self.distance("shownametags", 0.0))),
            row(tr!("rlva-modifier-sittp"), number(self.distance("sittp", DEFAULT_DISTANCE))),
            row(tr!("rlva-modifier-tplocal"), number(self.distance("tplocal", DEFAULT_DISTANCE))),
        ]
    }
}

/// Writes the texture of `@setcam_textures:<id>` through the asset id rule.
/// The Legacy Viewer lists it as it came; this is stricter, as for any asset
/// id that leaves the viewer through a window. `ids` is only taken when a row
/// has such an id: the list is refreshed every frame.
pub fn hide_texture_ids(rows: &mut [(Uuid, String)], ids: impl FnOnce() -> AssetIds) {
    let mut textures = rows
        .iter_mut()
        .filter_map(|(_, text)| {
            let (name, option) = text.split_once(':')?;
            if canonical(name) != "setcam_textures" {
                return None;
            }
            let id = Uuid::parse_str(option).ok()?;
            let option_start = name.len() + 1;
            Some((text, option_start, id))
        })
        .peekable();
    if textures.peek().is_none() {
        return;
    }
    let ids = ids();
    for (text, option_start, id) in textures {
        text.replace_range(option_start.., &ids.id(id).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Inventory;

    const OBJECT: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);
    const FRIEND: &str = "00000000-0000-0000-0000-00000000000a";
    const TEXTURE: &str = "00000000-0000-0000-0000-0000000000bb";

    fn with_rules(commands: &[(Uuid, &str)]) -> Rlva {
        let mut rlva = Rlva::default();
        rlva.configure(true);
        rlva.preferences.experimental_commands = true;
        for (source, command) in commands {
            let parsed = super::super::Command::parse(command).expect("a valid test command");
            rlva.process(*source, parsed);
        }
        rlva
    }

    fn value<'a>(rows: &'a [Modifier], name: &str) -> &'a Modifier {
        rows.iter().find(|row| row.name == name).expect("a listed modifier")
    }

    #[test]
    fn only_the_legacy_behaviours_list_their_options_as_exceptions() {
        let rlva = with_rules(&[
            (OBJECT, "recvim=n"),
            (OBJECT, &format!("recvim:{FRIEND}=add")),
            (OBJECT, &format!("sendim:{FRIEND}=add")),
            (OBJECT, &format!("shownames:{FRIEND}=add")),
            (OBJECT, "sendchannel:5=add"),
            (OBJECT, "jump=n"),
        ]);
        let exceptions: Vec<_> = rlva.exceptions().into_iter().map(|e| e.behaviour).collect();
        assert_eq!(exceptions, ["recvim", "sendim", "shownames"]);
        let behaviours: Vec<_> = rlva.behaviours().into_iter().map(|(_, text)| text).collect();
        assert_eq!(behaviours, ["jump", "recvim", "sendchannel:5"]);
        assert_eq!(rlva.restrictions().len(), 6, "the protocol's own list keeps every rule");
    }

    #[test]
    fn a_distance_option_lists_as_an_inactive_exception_and_sets_its_modifiers() {
        let rlva = with_rules(&[
            (OBJECT, "recvim=n"),
            (OBJECT, &format!("recvim:{FRIEND}=add")),
            (OBJECT, "recvim:20;50=n"),
            (OTHER, "recvim:30=n"),
        ]);
        let exceptions = rlva.exceptions();
        let active = |option: &str| exceptions.iter().find(|e| e.option == option).expect("listed").active;
        assert!(active(FRIEND));
        assert!(!active("20;50"));
        let rows = rlva.modifiers();
        let min = value(&rows, tr!("rlva-modifier-recvim-min"));
        assert_eq!(min.value, Some(ModifierValue::Text("30.000".into())), "the largest minimum applies");
        assert_eq!(min.source, Some(OTHER));
        let max = value(&rows, tr!("rlva-modifier-recvim-max"));
        assert_eq!(
            max.value,
            Some(ModifierValue::Text("50.000".into())),
            "`recvim:30` gives no maximum"
        );
        assert_eq!(max.source, Some(OBJECT));
    }

    #[test]
    fn every_modifier_is_listed_and_unset_ones_stay_at_their_default() {
        let rows = with_rules(&[]).modifiers();
        assert_eq!(rows.len(), 21, "RLV_MODIFIER_COUNT");
        assert!(rows.iter().all(|row| row.value.is_none() && row.source.is_none()));
    }

    #[test]
    fn distance_modifiers_apply_the_closest_and_a_bare_behaviour_adds_the_default() {
        let rlva = with_rules(&[
            (OBJECT, "fartouch=n"),
            (OTHER, "fartouch:0.5=n"),
            (OBJECT, "sittp=n"),
            (OBJECT, "tplocal:9=n"),
        ]);
        let rows = rlva.modifiers();
        let touch = value(&rows, tr!("rlva-modifier-fartouch"));
        assert_eq!(touch.value, Some(ModifierValue::Text("0.500".into())));
        assert_eq!(touch.source, Some(OTHER));
        assert_eq!(
            value(&rows, tr!("rlva-modifier-sittp")).value,
            Some(ModifierValue::Text("1.500".into()))
        );
        assert_eq!(
            value(&rows, tr!("rlva-modifier-tplocal")).value,
            Some(ModifierValue::Text("9.000".into()))
        );
    }

    #[test]
    fn nametag_exceptions_are_not_distances() {
        let rlva = with_rules(&[(OBJECT, &format!("shownametags:{FRIEND}=add"))]);
        assert_eq!(value(&rlva.modifiers(), tr!("rlva-modifier-nametags")).value, None);
        let rlva = with_rules(&[(OBJECT, "shownametags:5=n")]);
        assert_eq!(
            value(&rlva.modifiers(), tr!("rlva-modifier-nametags")).value,
            Some(ModifierValue::Text("5.000".into()))
        );
    }

    #[test]
    fn camera_modifiers_follow_the_exclusive_owner_like_the_camera_does() {
        let rlva = with_rules(&[
            (OBJECT, "setcam=n"),
            (OBJECT, "setcam_avdistmax:10=n"),
            (OTHER, "setcam_avdistmax:2=n"),
            (OBJECT, "setcam_eyeoffset:-4/0/1=n"),
        ]);
        let rows = rlva.modifiers();
        let max = value(&rows, tr!("rlva-modifier-avatar-max"));
        assert_eq!(max.value, Some(ModifierValue::Text("10.000".into())));
        assert_eq!(max.source, Some(OBJECT));
        assert_eq!(rlva.camera_limit("setcam_avdistmax", f32::MAX, false), 10.0);
        assert_eq!(
            value(&rows, tr!("rlva-modifier-eye-offset")).value,
            Some(ModifierValue::Text("-4/0/1".into()))
        );
    }

    #[test]
    fn a_forced_camera_value_has_no_issuing_row_and_a_bare_zoom_uses_the_default_view() {
        let mut rlva = with_rules(&[(OBJECT, "camzoommin=n")]);
        rlva.camera_values.insert("setcam_eyeoffsetscale".into(), "2".into());
        let rows = rlva.modifiers();
        let scale = value(&rows, tr!("rlva-modifier-eye-scale"));
        assert_eq!(scale.value, Some(ModifierValue::Text("2".into())));
        assert_eq!(scale.source, None);
        assert_eq!(
            value(&rows, tr!("rlva-modifier-fov-min")).value,
            Some(ModifierValue::Text(format!("{:.3}", std::f32::consts::FRAC_PI_3)))
        );
    }

    #[test]
    fn a_camera_texture_is_an_asset_and_a_disabled_rlva_lists_nothing() {
        let rlva_on = with_rules(&[(OBJECT, &format!("setcam_textures:{TEXTURE}=n")), (OBJECT, "fartouch=n")]);
        let id = Uuid::parse_str(TEXTURE).unwrap();
        assert_eq!(
            value(&rlva_on.modifiers(), tr!("rlva-modifier-texture")).value,
            Some(ModifierValue::Asset(id))
        );
        let mut off = rlva_on;
        off.configure(false);
        assert!(off.behaviours().is_empty());
        assert!(off.exceptions().is_empty());
        assert!(off.modifiers().iter().all(|row| row.value.is_none()));
    }

    #[test]
    fn a_camera_texture_id_only_shows_under_the_asset_id_rule() {
        let rows = || {
            vec![
                (OBJECT, format!("setcam_textures:{TEXTURE}")),
                (OBJECT, format!("camtextures:{TEXTURE}")),
                (OBJECT, "setcam_textures".to_owned()),
                (OBJECT, format!("detachthis:{TEXTURE}")),
            ]
        };
        let inventory = Inventory::default();
        let mut hidden = rows();
        hide_texture_ids(&mut hidden, || AssetIds::new(&inventory, [], false));
        assert!(hidden[0].1.starts_with("setcam_textures:hidden-"), "{}", hidden[0].1);
        assert!(hidden[1].1.starts_with("camtextures:hidden-"), "{}", hidden[1].1);
        assert_eq!(hidden[2].1, "setcam_textures");
        assert_eq!(hidden[3].1, format!("detachthis:{TEXTURE}"), "not a texture rule");

        let id = Uuid::parse_str(TEXTURE).unwrap();
        let mut local = rows();
        hide_texture_ids(&mut local, || AssetIds::new(&inventory, [id], false));
        assert_eq!(local, rows(), "the user's own local texture shows");
    }

    #[test]
    fn the_asset_id_rule_is_only_read_when_a_row_needs_it() {
        let mut rows = vec![(OBJECT, "jump".to_owned())];
        hide_texture_ids(&mut rows, || panic!("no row names a texture"));
    }
}
