//! Display sets follow the canonical Bento skeleton names. They never filter
//! goals: hiding a control must not disable the corresponding constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Body,
    Spine,
    Face,
    Arms,
    Legs,
    Fingers,
    Wings,
    Tail,
    Hind,
    Other,
}

impl Group {
    const ALL: [Self; 10] = [
        Self::Body,
        Self::Spine,
        Self::Face,
        Self::Arms,
        Self::Legs,
        Self::Fingers,
        Self::Wings,
        Self::Tail,
        Self::Hind,
        Self::Other,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Body => "ik-edit-set-body",
            Self::Spine => "ik-edit-set-spine",
            Self::Face => "ik-edit-set-face",
            Self::Arms => "ik-edit-set-arms",
            Self::Legs => "ik-edit-set-legs",
            Self::Fingers => "ik-edit-set-fingers",
            Self::Wings => "ik-edit-set-wings",
            Self::Tail => "ik-edit-set-tail",
            Self::Hind => "ik-edit-set-hind",
            Self::Other => "ik-edit-set-other",
        }
    }
    pub fn of(name: &str) -> Self {
        if name.starts_with("mFace") || name.starts_with("mEye") {
            Self::Face
        } else if name.starts_with("mHand") {
            Self::Fingers
        } else if name.starts_with("mWing") {
            Self::Wings
        } else if name.starts_with("mTail") {
            Self::Tail
        } else if name.starts_with("mHind") {
            Self::Hind
        } else if ["mCollar", "mShoulder", "mElbow", "mWrist"].iter().any(|p| name.starts_with(p)) {
            Self::Arms
        } else if ["mHip", "mKnee", "mAnkle", "mFoot", "mToe"].iter().any(|p| name.starts_with(p)) {
            Self::Legs
        } else if name.starts_with("mSpine") {
            Self::Spine
        } else if ["mPelvis", "mTorso", "mChest", "mNeck", "mHead", "mSkull"].contains(&name) {
            Self::Body
        } else {
            Self::Other
        }
    }
}

pub(super) struct Visibility {
    enabled: [bool; 10],
}
impl Default for Visibility {
    fn default() -> Self {
        let mut display = Self { enabled: [false; 10] };
        display.body();
        display
    }
}
impl Visibility {
    pub fn reveal(&mut self, bone: &str) {
        self.enabled[Group::of(bone) as usize] = true;
    }
    pub fn only(&mut self, group: Group) {
        self.enabled.fill(false);
        self.enabled[group as usize] = true;
    }
    pub fn visible(&self, name: &str) -> bool {
        self.enabled[Group::of(name) as usize]
    }
    /// Hidden Bento spine pairs must not leave ghost segments/duplicate pivots
    /// on the human torso. Keep their actual transforms in the rig and solver.
    pub fn parent(&self, rig: &crate::scene::anim::Rig, joint: usize) -> Option<usize> {
        let mut parent = rig.parent.get(joint).copied().flatten();
        for _ in 0..rig.len() {
            let p = parent?;
            let name = &rig.canonical_names[p];
            if self.visible(name) {
                return Some(p);
            }
            if Group::of(name) != Group::Spine {
                return None;
            }
            parent = rig.parent[p];
        }
        None
    }
    pub fn body(&mut self) {
        self.enabled = [false; 10];
        for group in [Group::Body, Group::Arms, Group::Legs] {
            self.enabled[group as usize] = true;
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, palette: &crate::theme::Palette) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.strong(aurora_i18n::tr!("ik-edit-visible-bones"))
                .on_hover_text(aurora_i18n::tr!("ik-edit-visibility-hint"));
            if ui.button(aurora_i18n::tr!("ik-edit-show-all")).clicked() {
                self.enabled.fill(true);
                changed = true;
            }
            if ui.button(aurora_i18n::tr!("ik-edit-show-none")).clicked() {
                self.enabled.fill(false);
                changed = true;
            }
            if ui.button(aurora_i18n::tr!("ik-edit-body-preset")).clicked() {
                self.body();
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            for group in Group::ALL {
                let enabled = &mut self.enabled[group as usize];
                changed |= crate::ui::widgets::checkbox(ui, palette, enabled, aurora_i18n::text(group.label())).changed();
            }
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ik_edit::{Editor, Frame};
    use glam::{Mat4, Vec3};
    use std::time::Instant;
    #[test]
    fn canonical_bento_sets_cover_the_rig_and_visibility_never_changes_constraints() {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = &lib.rig;
        let mut visibility = Visibility::default();
        assert!(visibility.visible("mTorso") && visibility.visible("mChest"));
        assert!(!visibility.visible("mSpine1") && !visibility.visible("mSpine4"));
        assert!(!visibility.visible("mGroin"));
        assert!(!visibility.visible("mWingsRoot") && !visibility.visible("mTail1") && !visibility.visible("mHindLimbsRoot"));
        assert_eq!(visibility.parent(rig, rig.names["mTorso"]), Some(rig.pelvis));
        assert_eq!(visibility.parent(rig, rig.names["mChest"]), Some(rig.names["mTorso"]));
        visibility.enabled.fill(true);
        for name in &rig.canonical_names {
            assert!(visibility.visible(name));
        }
        for (name, group) in [
            ("mFaceJaw", Group::Face),
            ("mEyeLeft", Group::Face),
            ("mCollarRight", Group::Arms),
            ("mWristLeft", Group::Arms),
            ("mAnkleRight", Group::Legs),
            ("mHandThumb2Left", Group::Fingers),
            ("mWingsRoot", Group::Wings),
            ("mWing4FanRight", Group::Wings),
            ("mTail6", Group::Tail),
            ("mHindLimbsRoot", Group::Hind),
            ("mHindLimb4Right", Group::Hind),
            ("mSpine4", Group::Spine),
        ] {
            assert!(rig.names.contains_key(name));
            assert_eq!(Group::of(name), group);
        }
        for group in Group::ALL.into_iter().filter(|g| *g != Group::Other) {
            assert!(rig.canonical_names.iter().any(|n| Group::of(n) == group));
        }
        visibility.body();
        assert!(!visibility.visible("mSpine1"));
        visibility.reveal("mSpine1");
        assert_eq!(visibility.parent(rig, rig.names["mTorso"]), Some(rig.names["mSpine2"]));
        visibility.body();
        assert!(visibility.visible("mHead") && visibility.visible("mWristLeft") && visibility.visible("mAnkleRight"));
        assert!(!visibility.visible("mFaceJaw") && !visibility.visible("mHandThumb1Left") && !visibility.visible("mWing1Left"));
        let frame = Frame {
            model: Mat4::IDENTITY,
            joints: rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
            at: Instant::now(),
        };
        let mut editor = Editor::default();
        editor.toggle();
        editor.enable(rig.names["mWristLeft"], true, rig, &frame);
        editor.visibility.enabled.fill(false);
        assert_eq!(editor.request().goals.len(), 1);
        assert!(editor.enabled(rig.names["mWristLeft"]));
    }
}
