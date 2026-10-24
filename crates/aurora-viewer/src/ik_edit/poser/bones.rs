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
/// Whether a paired joint is the left one (`Some(true)`), the right one, or
/// unpaired.
pub(super) fn side_of(name: &str) -> Option<bool> {
    stem(name).1
}
/// The finger rows of the Hands grid, in the window's order, with their
/// Bento stem and label.
pub(super) const FINGERS: [(&str, &str); 5] = [
    ("mHandThumb", "poser-finger-thumb"),
    ("mHandIndex", "poser-finger-index"),
    ("mHandMiddle", "poser-finger-middle"),
    ("mHandRing", "poser-finger-ring"),
    ("mHandPinky", "poser-finger-pinky"),
];
/// The bone of a finger cell: the finger's stem, its joint (1 base, 3
/// tip) on the chosen side, the left one in Mirror.
pub(super) fn finger_bone(rig: &Rig, stem: &str, joint: usize, side: Side) -> Option<usize> {
    let suffix = if side == Side::Right { "Right" } else { "Left" };
    rig.names.get(&format!("{stem}{joint}{suffix}")).copied()
}
/// The hand presets: their label and icon, in `hand_preset`'s order.
const PRESETS: [(&str, &str); 5] = [
    ("poser-hand-relaxed", "hand-move"),
    ("poser-hand-fist", "hand-grab"),
    ("poser-hand-open", "hand-stop"),
    ("poser-hand-point", "hand-finger"),
    ("poser-hand-pinch", "hand-click"),
];
/// Room of a list row and a finger cell.
const ROW_H: f32 = 28.0;
const CELL_H: f32 = 30.0;
/// The edited joint's dot (the world markers are 2.5 and 4.5).
const DOT: f32 = 3.0;

/// Tabs sharing one track as wide as the space left, the chosen one lit in
/// `raised`: the Côté selector (`widgets::segmented` fits its content).
fn stretched_choice(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, options: &[(&str, &str)]) -> bool {
    let height = 28.0;
    let (track, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    ui.painter()
        .rect_filled(track, ui.visuals().widgets.inactive.corner_radius, p.field);
    let inner = track.shrink(2.0);
    let width = inner.width() / options.len().max(1) as f32;
    let mut changed = false;
    for (i, (label, tip)) in options.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(inner.left() + width * i as f32, inner.top()),
            egui::vec2(width, inner.height()),
        );
        let response = ui.interact(rect, ui.scope_id().with(("poser-side", i)), Sense::click());
        let on = *selected == i;
        if on || response.hovered() {
            let face = if on { p.raised } else { p.raised.gamma_multiply(0.5) };
            ui.painter().rect_filled(rect, widgets::inner_radius(ui.visuals()), face);
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            egui::FontId::proportional(12.5),
            if on { p.ink } else { p.muted },
        );
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, *label));
        if response.on_hover_text(*tip).clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}
/// A small capitals caption over a group of controls.
pub(super) fn caption(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(10.5).color(p.muted));
}

impl Editor {
    /// The side words of the window: "left", "right" or "both".
    pub(super) fn side_word(&self) -> &'static str {
        aurora_i18n::text(match self.poser.side {
            Side::Left => "poser-side-word-left",
            Side::Right => "poser-side-word-right",
            Side::Mirror => "poser-side-word-both",
        })
    }
    /// The Côté choice: Mirror is the editor's symmetry, and a paired joint
    /// already selected moves to the chosen side.
    pub(super) fn set_pose_side(&mut self, side: Side, rig: &Rig, frame: &Frame) {
        self.mirrors.initialize(rig);
        self.poser.side = side;
        self.symmetry = side == Side::Mirror;
        if let Some(bone) = self.selected
            && let Some(left) = side_of(&rig.canonical_names[bone])
            && let Some(other) = self.mirrors.counterpart(bone)
            && (side == Side::Right && left || side == Side::Left && !left)
        {
            self.select_pose_bone(other, rig, frame);
        }
    }
    /// A joint picked elsewhere (a world marker) shows its side in the
    /// selector; Mirror stays as chosen.
    pub(super) fn follow_selected_side(&mut self, rig: &Rig) {
        if self.poser.side != Side::Mirror
            && let Some(left) = self.selected.and_then(|b| side_of(&rig.canonical_names[b]))
        {
            self.poser.side = if left { Side::Left } else { Side::Right };
        }
        self.symmetry = self.poser.side == Side::Mirror;
    }
    /// The bone of a row on the chosen side (the left one in Mirror).
    fn row_bone(&self, pair: [Option<usize>; 2]) -> Option<usize> {
        match self.poser.side {
            Side::Right => pair[1].or(pair[0]),
            _ => pair[0].or(pair[1]),
        }
    }
    fn subject_edited(&self, bone: usize) -> bool {
        self.poser.sessions.get(&self.poser.subject).is_some_and(|s| s.edited(bone))
    }
    pub(super) fn side_selector(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(34.0, 28.0),
                egui::Label::new(egui::RichText::new(aurora_i18n::text("poser-side")).color(p.muted)),
            );
            let mut index = self.poser.side as usize;
            let options = [
                (aurora_i18n::text("poser-side-left"), aurora_i18n::text("poser-side-left-tip")),
                (aurora_i18n::text("poser-side-right"), aurora_i18n::text("poser-side-right-tip")),
                (aurora_i18n::text("poser-side-mirror"), aurora_i18n::text("poser-side-mirror-tip")),
            ];
            if stretched_choice(ui, p, &mut index, &options) {
                let side = [Side::Left, Side::Right, Side::Mirror][index];
                self.set_pose_side(side, rig, frame);
            }
        });
    }
    /// One row per joint of the active tab (one per pair: the Côté selector
    /// chooses the side), with the edited dot and the Bento names as tooltip.
    pub(super) fn bone_list(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        self.mirrors.initialize(rig);
        let filter = self.poser.filter.to_lowercase();
        if self.poser.group == 0 {
            caption(ui, p, aurora_i18n::text("poser-whole-avatar"));
            let label = aurora_i18n::text("poser-avatar-position");
            if filter.is_empty() || label.to_lowercase().contains(&filter) {
                let edited = self.subject_edited(rig.pelvis) && self.poser.transform == 1;
                if self.joint_row(
                    ui,
                    p,
                    label,
                    self.selected == Some(rig.pelvis) && self.poser.transform == 1,
                    edited,
                    "mPelvis",
                ) {
                    self.select_pose_bone(rig.pelvis, rig, frame);
                    self.poser.transform = 1;
                    self.tool = Tool::Move;
                }
            }
        }
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
                ui.add_space(4.0);
                caption(
                    ui,
                    p,
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
            let Some(bone) = self.row_bone(pair) else { continue };
            let both: Vec<_> = pair.iter().flatten().copied().collect();
            let selected = self.selected.is_some_and(|s| both.contains(&s)) && !(bone == rig.pelvis && self.poser.transform == 1);
            let edited = match self.poser.side {
                Side::Mirror => both.iter().any(|b| self.subject_edited(*b)),
                _ => self.subject_edited(bone),
            };
            let names = both
                .iter()
                .map(|b| rig.canonical_names[*b].as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            if self.joint_row(ui, p, &label(&base), selected, edited, &names) {
                self.select_pose_bone(bone, rig, frame);
            }
        }
    }
    /// A selectable joint row; true when clicked.
    fn joint_row(&self, ui: &mut egui::Ui, p: &Palette, label: &str, selected: bool, edited: bool, tip: &str) -> bool {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), Sense::click());
        let hovered = response.hovered();
        if selected || hovered {
            let face = if selected { p.violet } else { p.raised.gamma_multiply(0.5) };
            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
        }
        let ink = if selected { p.on_accent } else { p.ink };
        widgets::paint_elided(
            ui,
            label,
            egui::pos2(rect.left() + 10.0, rect.center().y),
            rect.width() - 30.0,
            egui::FontId::proportional(13.0),
            ink,
        );
        if edited {
            let dot = if selected { p.on_accent } else { p.violet_light };
            ui.painter()
                .circle_filled(egui::pos2(rect.right() - 10.0, rect.center().y), DOT, dot);
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
        response.on_hover_text(tip).clicked()
    }
    /// The Hands tab: the five presets, applied to the chosen side (both in
    /// Mirror), and the finger grid (finger × base, middle, tip).
    pub(super) fn hands_panel(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let side = self.side_word();
        caption(ui, p, &aurora_i18n::tr!("poser-presets-for", side = side));
        let width = (ui.available_width() - 4.0 * 4.0) / 5.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (index, (key, icon)) in PRESETS.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 46.0), Sense::click());
                let face = if response.hovered() { p.raised } else { p.bar };
                ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
                if let Some(texture) = crate::ui::icons::global(icon) {
                    let at = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 15.0), egui::Vec2::splat(16.0));
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(texture.id(), at, uv, p.violet_light);
                }
                widgets::paint_elided(
                    ui,
                    aurora_i18n::text(key),
                    egui::pos2(rect.left() + 2.0, rect.bottom() - 11.0),
                    rect.width() - 4.0,
                    egui::FontId::proportional(10.0),
                    p.ink,
                );
                let label = aurora_i18n::text(key);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
                if response.on_hover_text(label).clicked() {
                    self.poser.preset = index;
                    let sides = match self.poser.side {
                        Side::Left => Some(true),
                        Side::Right => Some(false),
                        Side::Mirror => None,
                    };
                    self.hand_preset(rig, sides);
                }
            }
        });
        ui.add_space(6.0);
        caption(ui, p, &aurora_i18n::tr!("poser-fingers-for", side = side));
        let label_w = 80.0;
        let cell_w = ((ui.available_width() - label_w - 3.0 * 4.0) / 3.0).max(20.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(label_w);
            for key in ["poser-finger-col-base", "poser-finger-col-middle", "poser-finger-col-tip"] {
                ui.add_sized(
                    egui::vec2(cell_w, 14.0),
                    egui::Label::new(egui::RichText::new(aurora_i18n::text(key)).size(11.0).color(p.muted)),
                );
            }
        });
        let filter = self.poser.filter.to_lowercase();
        for (stem, key) in FINGERS {
            let name = aurora_i18n::text(key);
            if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !stem.to_lowercase().contains(&filter) {
                continue;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(label_w, CELL_H), Sense::hover());
                widgets::paint_elided(
                    ui,
                    name,
                    egui::pos2(rect.left(), rect.center().y),
                    label_w - 4.0,
                    egui::FontId::proportional(12.5),
                    p.ink,
                );
                for joint in 1..=3 {
                    let Some(bone) = finger_bone(rig, stem, joint, self.poser.side) else {
                        ui.allocate_exact_size(egui::vec2(cell_w, CELL_H), Sense::hover());
                        continue;
                    };
                    let other = self.mirrors.counterpart(bone);
                    let selected =
                        self.selected == Some(bone) || self.poser.side == Side::Mirror && self.selected.is_some() && self.selected == other;
                    let edited =
                        self.subject_edited(bone) || self.poser.side == Side::Mirror && other.is_some_and(|o| self.subject_edited(o));
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(cell_w, CELL_H), Sense::click());
                    let face = if selected {
                        p.violet
                    } else if response.hovered() {
                        p.raised
                    } else {
                        p.bar
                    };
                    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
                    let dot = match (selected, edited) {
                        (true, _) => p.on_accent,
                        (false, true) => p.violet_light,
                        (false, false) => p.raised,
                    };
                    ui.painter().circle_filled(rect.center(), DOT, dot);
                    let label = aurora_i18n::tr!("poser-numbered-bone", name = name, number = joint).to_string();
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &label));
                    if response.on_hover_text(&rig.canonical_names[bone]).clicked() {
                        self.select_pose_bone(bone, rig, frame);
                    }
                }
            });
        }
    }
}
