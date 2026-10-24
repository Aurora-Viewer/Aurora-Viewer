//! Canonical paired Bento/collision rows. Technical names remain data/tooltips.
use super::*;
use std::collections::BTreeMap;

fn stem(name: &str) -> (&str, Option<bool>) {
    if let Some(s) = name.strip_suffix("Left") {
        (s, Some(true))
    } else if let Some(s) = name.strip_suffix("Right") {
        (s, Some(false))
    } else if let Some(s) = name.strip_prefix("L_") {
        (s, Some(true))
    } else if let Some(s) = name.strip_prefix("R_") {
        (s, Some(false))
    } else if let Some(s) = name.strip_prefix("LEFT_") {
        (s, Some(true))
    } else if let Some(s) = name.strip_prefix("RIGHT_") {
        (s, Some(false))
    } else {
        (name, None)
    }
}
pub(super) fn label(name: &str) -> String {
    let (base, _) = stem(name);
    for (prefix, key) in [
        ("mHandThumb", "poser-finger-thumb"),
        ("mHandIndex", "poser-finger-index"),
        ("mHandMiddle", "poser-finger-middle"),
        ("mHandRing", "poser-finger-ring"),
        ("mHandPinky", "poser-finger-pinky"),
        ("mSpine", "poser-bone-spine"),
        ("mWing", "poser-bone-wing"),
        ("mTail", "poser-bone-tail"),
        ("mHindLimb", "poser-bone-hind"),
    ] {
        if let Some(number) = base.strip_prefix(prefix).filter(|n| n.parse::<usize>().is_ok()) {
            return aurora_i18n::tr!("poser-numbered-bone", name = aurora_i18n::text(key), number = number).to_string();
        }
    }
    let key = match base {
        "mSkull" => "poser-bone-skull",
        "mEye" => "poser-bone-eye",
        "mFaceRoot" => "poser-bone-face-root",
        "mFaceEyeAlt" => "poser-bone-eye-alt",
        "mFaceForehead" => "poser-bone-forehead",
        "mFaceForeheadCenter" => "poser-bone-forehead-center",
        "mFaceEyebrowOuter" => "poser-bone-brow-outer",
        "mFaceEyebrowCenter" => "poser-bone-brow-center",
        "mFaceEyebrowInner" => "poser-bone-brow-inner",
        "mFaceEyeLidUpper" => "poser-bone-lid-upper",
        "mFaceEyeLidLower" => "poser-bone-lid-lower",
        "mFaceEar1" => "poser-bone-ear-base",
        "mFaceEar2" => "poser-bone-ear-tip",
        "mFaceNose" => "poser-bone-nose",
        "mFaceNoseCenter" => "poser-bone-nose-center",
        "mFaceNoseBase" => "poser-bone-nose-base",
        "mFaceNoseBridge" => "poser-bone-nose-bridge",
        "mFaceCheekLower" => "poser-bone-cheek-lower",
        "mFaceCheekUpper" => "poser-bone-cheek-upper",
        "mFaceJaw" => "poser-bone-jaw",
        "mFaceChin" => "poser-bone-chin",
        "mFaceJawShaper" => "poser-bone-jaw-shaper",
        "mFaceTeethLower" => "poser-bone-teeth-lower",
        "mFaceTeethUpper" => "poser-bone-teeth-upper",
        "mFaceLipLower" => "poser-bone-lip-lower",
        "mFaceLipUpper" => "poser-bone-lip-upper",
        "mFaceLipLowerCenter" => "poser-bone-lip-lower-center",
        "mFaceLipUpperCenter" => "poser-bone-lip-upper-center",
        "mFaceLipCorner" => "poser-bone-lip-corner",
        "mFaceTongueBase" => "poser-bone-tongue-base",
        "mFaceTongueTip" => "poser-bone-tongue-tip",
        "mFaceEyecornerInner" => "poser-bone-eye-corner",
        "mWingsRoot" => "poser-bone-wings-root",
        "mWing4Fan" => "poser-bone-wing-fan",
        "mGroin" => "poser-bone-groin",
        "mHindLimbsRoot" => "poser-bone-hind-root",
        "PELVIS" => "poser-bone-volume-pelvis",
        "BUTT" => "poser-bone-volume-butt",
        "BELLY" => "poser-bone-volume-belly",
        "HANDLE" => "poser-bone-volume-handle",
        "LOWER_BACK" => "poser-bone-volume-lower-back",
        "CHEST" => "poser-bone-volume-chest",
        "PEC" => "poser-bone-volume-pec",
        "UPPER_BACK" => "poser-bone-volume-upper-back",
        "NECK" => "poser-bone-volume-neck",
        "HEAD" => "poser-bone-volume-head",
        "CLAVICLE" => "poser-bone-volume-collar",
        "UPPER_ARM" => "poser-bone-volume-upper-arm",
        "LOWER_ARM" => "poser-bone-volume-lower-arm",
        "HAND" => "poser-bone-volume-hand",
        "UPPER_LEG" => "poser-bone-volume-upper-leg",
        "LOWER_LEG" => "poser-bone-volume-lower-leg",
        "FOOT" => "poser-bone-volume-foot",
        "mPelvis" => "poser-bone-pelvis",
        "mTorso" => "poser-bone-torso",
        "mChest" => "poser-bone-chest",
        "mNeck" => "poser-bone-neck",
        "mHead" => "poser-bone-head",
        "mCollar" => "poser-bone-collar",
        "mShoulder" => "poser-bone-shoulder",
        "mElbow" => "poser-bone-elbow",
        "mWrist" => "poser-bone-wrist",
        "mHip" => "poser-bone-hip",
        "mKnee" => "poser-bone-knee",
        "mAnkle" => "poser-bone-ankle",
        "mFoot" => "poser-bone-foot",
        "mToe" => "poser-bone-toe",
        _ => return base.to_owned(),
    };
    aurora_i18n::text(key).to_owned()
}
fn section(name: &str) -> usize {
    use visibility::Group;
    match visibility::Group::of(name) {
        Group::Body => 0,
        Group::Arms => 1,
        Group::Legs => 2,
        Group::Face => 3,
        Group::Fingers => match stem(name).0.chars().last() {
            Some('1') => 10,
            Some('2') => 11,
            _ => 12,
        },
        Group::Spine => 5,
        Group::Wings => 6,
        Group::Tail => 7,
        Group::Hind => 8,
        Group::Other => 9,
    }
}
impl Editor {
    pub(super) fn bone_list(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        if self.poser.group == 0 {
            ui.strong(aurora_i18n::tr!("poser-whole-avatar"));
            if ui
                .selectable_label(self.selected == Some(rig.pelvis), aurora_i18n::tr!("poser-avatar-position"))
                .clicked()
            {
                self.select(rig.pelvis, rig, frame);
                self.poser.transform = 1;
                self.tool = Tool::Move;
            }
        }
        if self.poser.group == 2 {
            ui.label(aurora_i18n::tr!("poser-hand-presets"));
            egui::ComboBox::from_id_salt("poser-hand-preset")
                .selected_text(aurora_i18n::text(
                    [
                        "poser-hand-relaxed",
                        "poser-hand-fist",
                        "poser-hand-open",
                        "poser-hand-point",
                        "poser-hand-pinch",
                    ][self.poser.preset],
                ))
                .show_ui(ui, |ui| {
                    for (i, key) in [
                        "poser-hand-relaxed",
                        "poser-hand-fist",
                        "poser-hand-open",
                        "poser-hand-point",
                        "poser-hand-pinch",
                    ]
                    .iter()
                    .enumerate()
                    {
                        ui.selectable_value(&mut self.poser.preset, i, aurora_i18n::text(key));
                    }
                });
            ui.horizontal_wrapped(|ui| {
                for (left, key) in [(true, "poser-apply-left"), (false, "poser-apply-right")] {
                    if ui.button(aurora_i18n::text(key)).clicked() {
                        self.hand_preset(rig, left);
                    }
                }
            });
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.poser.filter)
                .hint_text(aurora_i18n::tr!("poser-search"))
                .desired_width(ui.available_width()),
        );
        let filter = self.poser.filter.to_lowercase();
        let mut rows = BTreeMap::<(usize, usize, String), [Option<usize>; 2]>::new();
        for (bone, name) in rig.canonical_names.iter().enumerate() {
            if group(name) != self.poser.group {
                continue;
            }
            let (base, side) = stem(name);
            let label = label(name);
            if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !label.to_lowercase().contains(&filter) {
                continue;
            }
            let rank = self.mirrors.counterpart(bone).map_or(bone, |other| bone.min(other));
            let key = (section(name), rank, base.to_owned());
            let row = rows.entry(key).or_insert([None, None]);
            row[usize::from(side == Some(false))] = Some(bone);
        }
        let mut section_id = None;
        for ((section, _, base), pair) in rows {
            if section_id != Some(section) {
                section_id = Some(section);
                ui.colored_label(
                    p.muted,
                    aurora_i18n::text(
                        [
                            "poser-section-body",
                            "poser-section-arms",
                            "poser-section-legs",
                            "poser-section-face",
                            "poser-section-fingers",
                            "ik-edit-set-spine",
                            "ik-edit-set-wings",
                            "ik-edit-set-tail",
                            "ik-edit-set-hind",
                            "poser-section-physics",
                            "poser-section-finger-base",
                            "poser-section-finger-middle",
                            "poser-section-finger-tip",
                        ][section],
                    ),
                );
            }
            ui.horizontal(|ui| {
                if pair[1].is_some() {
                    ui.add_sized(
                        egui::vec2((ui.available_width() - 78.0).max(50.0), 25.0),
                        egui::Label::new(label(&base)).truncate(),
                    );
                    for (side, bone) in pair.into_iter().enumerate() {
                        if let Some(bone) = bone {
                            let response = ui.add_sized(
                                egui::vec2(30.0, 25.0),
                                egui::Button::new(aurora_i18n::text(if side == 0 { "poser-left-short" } else { "poser-right-short" }))
                                    .selected(self.selected == Some(bone)),
                            );
                            if response.clicked() {
                                self.select(bone, rig, frame);
                            }
                            if self.poser.sessions.get(&self.poser.subject).is_some_and(|s| s.edited(bone)) {
                                ui.painter()
                                    .circle_filled(response.rect.right_top() + egui::vec2(-4.0, 4.0), 2.0, p.violet_light);
                            }
                            response.on_hover_text(&rig.canonical_names[bone]);
                        }
                    }
                } else if let Some(bone) = pair[0] {
                    let response = ui.selectable_label(self.selected == Some(bone), label(&base));
                    if response.clicked() {
                        self.select(bone, rig, frame);
                    }
                    if self.poser.sessions.get(&self.poser.subject).is_some_and(|s| s.edited(bone)) {
                        ui.painter()
                            .circle_filled(response.rect.right_center() - egui::vec2(4.0, 0.0), 2.0, p.violet_light);
                    }
                    response.on_hover_text(&rig.canonical_names[bone]);
                }
            });
        }
    }
}
