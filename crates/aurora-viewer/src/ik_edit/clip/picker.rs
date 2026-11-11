//! The animation editor's bone picker: a clickable schematic of the avatar's
//! own rest skeleton, joints as dots with a line to their parent, in a Body,
//! Hands or Face view. It is drawn from the rig, never from artwork, so it
//! follows whatever skeleton the avatar library loads.
use super::*;
use crate::ik_edit::visibility::Group;

/// The hit radius of a dot: a target of at least 16 px.
const HIT: f32 = 8.0;
const _: () = assert!(HIT * 2.0 >= 14.0, "a dot's target stays at least 14 px wide");
/// Room kept around the figure for the enlarged and ringed dots.
const MARGIN: f32 = 12.0;
/// How far the front views look down on the avatar. A joint ahead of another
/// drops a little on screen, so the toe no longer hides behind its foot, nor
/// the nose behind the face.
const ELEVATION: f32 = 0.5;

/// The picker's views, in the order of its choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum View {
    #[default]
    Body,
    Hands,
    Face,
}

impl View {
    pub(super) const ALL: [Self; 3] = [Self::Body, Self::Hands, Self::Face];
    fn label(self) -> &'static str {
        match self {
            Self::Body => "anim-picker-body",
            Self::Hands => "anim-picker-hands",
            Self::Face => "anim-picker-face",
        }
    }
    fn index(self) -> usize {
        Self::ALL.iter().position(|view| *view == self).unwrap_or(0)
    }
}

/// Whether a joint is drawn in a view. Body holds the main bones from the
/// pelvis to the head, the arms and the legs; the Bento spine pivots (doubles
/// of the pelvis and torso), the skull, fingers, face, wings, tail and hind
/// limbs stay out. Hands holds the finger joints, hung from their wrist; Face
/// the face joints and the eyes.
pub(super) fn shows(view: View, name: &str) -> bool {
    let group = Group::of(name);
    match view {
        View::Body => matches!(group, Group::Body | Group::Arms | Group::Legs) && name != "mSkull",
        View::Hands => group == Group::Fingers || name.starts_with("mWrist"),
        View::Face => group == Group::Face,
    }
}

/// Picker axes of a rest position in avatar space, where the avatar faces +X,
/// Z is up and +Y is its left. Body and Face look at the avatar from the
/// front (slightly above), so its left side lands on the picker's right.
/// Hands look down on the hands of the rest pose, the avatar's front toward
/// the bottom: the fingers a front view would stack spread apart.
pub(super) fn project(view: View, at: Vec3) -> egui::Vec2 {
    match view {
        View::Body | View::Face => egui::vec2(at.y, -at.z + ELEVATION * at.x),
        View::Hands => egui::vec2(at.y, at.x),
    }
}

/// One joint of a view at its projected rest position (metres, not fitted
/// yet), with the index of the dot of its nearest drawn ancestor.
#[derive(Clone, Debug)]
pub(super) struct Dot {
    pub bone: usize,
    pub at: egui::Vec2,
    pub parent: Option<usize>,
    /// On the avatar's left: Hands fits each hand into its own half.
    pub left: bool,
}

/// The dots of a view, parents before children, from the rest joint matrices
/// in avatar space (`Rig::world_with` with the rest rotations and positions).
pub(super) fn dots(rig: &Rig, rest: &[Mat4], view: View) -> Vec<Dot> {
    let mut index: Vec<Option<usize>> = vec![None; rig.len()];
    let mut dots = Vec::new();
    for &bone in &rig.order {
        let (Some(name), Some(matrix)) = (rig.canonical_names.get(bone), rest.get(bone)) else {
            continue;
        };
        if !shows(view, name) {
            continue;
        }
        // the line goes to the nearest ancestor the view draws
        let mut parent = None;
        let mut ancestor = rig.parent.get(bone).copied().flatten();
        for _ in 0..rig.len() {
            let Some(joint) = ancestor else { break };
            if let Some(dot) = index.get(joint).copied().flatten() {
                parent = Some(dot);
                break;
            }
            ancestor = rig.parent.get(joint).copied().flatten();
        }
        if let Some(slot) = index.get_mut(bone) {
            *slot = Some(dots.len());
        }
        dots.push(Dot {
            bone,
            at: project(view, matrix.w_axis.truncate()),
            parent,
            left: name.ends_with("Left"),
        });
    }
    dots
}

/// Screen positions of the dots, fitted into `rect` with one scale so the
/// figure keeps its proportions at any width, and centred. Hands fit each
/// hand into its own half: the avatar's left hand on the right.
pub(super) fn fit(view: View, dots: &[Dot], rect: egui::Rect) -> Vec<egui::Pos2> {
    let mut points = vec![rect.center(); dots.len()];
    if view == View::Hands {
        let middle = rect.center().x;
        let gap = egui::vec2(MARGIN * 0.5, 0.0);
        let screen_left = egui::Rect::from_min_max(rect.min, egui::pos2(middle, rect.max.y)).shrink2(gap);
        let screen_right = egui::Rect::from_min_max(egui::pos2(middle, rect.min.y), rect.max).shrink2(gap);
        place(dots, |dot| !dot.left, screen_left, &mut points);
        place(dots, |dot| dot.left, screen_right, &mut points);
    } else {
        place(dots, |_| true, rect, &mut points);
    }
    points
}

fn place(dots: &[Dot], keep: impl Fn(&Dot) -> bool, rect: egui::Rect, points: &mut [egui::Pos2]) {
    let kept = || dots.iter().enumerate().filter(|(_, dot)| keep(dot));
    let Some((min, max)) = kept().fold(None, |bounds: Option<(egui::Vec2, egui::Vec2)>, (_, dot)| {
        Some(bounds.map_or((dot.at, dot.at), |(min, max)| (min.min(dot.at), max.max(dot.at))))
    }) else {
        return;
    };
    let size = (max - min).max(egui::Vec2::splat(1e-4));
    let scale = (rect.width() / size.x).min(rect.height() / size.y).max(0.0);
    let middle = (min + max) * 0.5;
    for (index, dot) in kept() {
        points[index] = rect.center() + (dot.at - middle) * scale;
    }
}

/// The bones the user keyed at a frame. The hidden rest-pose keys a fresh
/// draft seeds on every bone at frame 0 (unauthored tracks) do not count.
pub(super) fn keyed_at(tracks: &[TimelineTrack], frame: u32) -> HashSet<usize> {
    tracks
        .iter()
        .filter(|track| track.authored && track.keys.binary_search(&frame).is_ok())
        .map(|track| track.bone)
        .collect()
}

/// The dot under the pointer: the nearest one within the hit radius.
pub(super) fn hit(points: &[egui::Pos2], pointer: egui::Pos2, radius: f32) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| (index, point.distance_sq(pointer)))
        .filter(|(_, distance)| *distance <= radius * radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// The picker's view and the dots of the three views, built once per rig.
#[derive(Default)]
pub(super) struct Picker {
    pub view: View,
    figures: Option<(Arc<Rig>, Arc<[Vec<Dot>; 3]>)>,
}

impl Picker {
    fn figures(&mut self, rig: &Arc<Rig>) -> Arc<[Vec<Dot>; 3]> {
        if let Some((cached, figures)) = &self.figures
            && Arc::ptr_eq(cached, rig)
        {
            return figures.clone();
        }
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
        let figures = Arc::new(View::ALL.map(|view| dots(rig, &rest, view)));
        self.figures = Some((rig.clone(), figures.clone()));
        figures
    }
}

impl Editor {
    /// Pick the bone to pose on a schematic of the avatar's skeleton. The
    /// selected bone is teal; a violet ring marks the bones keyed on the
    /// current frame.
    pub(in crate::ik_edit) fn bone_picker_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        let mut view = self.clip.picker.view.index();
        let labels: Vec<_> = View::ALL.iter().map(|view| (aurora_i18n::text(view.label()), true)).collect();
        if widgets::segmented(ui, palette, &mut view, &labels) {
            self.clip.picker.view = View::ALL[view];
        }
        let view = self.clip.picker.view;
        let figures = self.clip.picker.figures(rig);
        let dots = &figures[view.index()];
        let width = ui.available_width().max(1.0);
        let height = (width * 0.6).clamp(180.0, 260.0);
        let (rect, mut response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        let points = fit(view, dots, rect.shrink(MARGIN));
        let hovered = response.hover_pos().and_then(|pointer| hit(&points, pointer, HIT));
        let painter = ui.painter_at(rect);
        // the sides as seen from the front: the avatar's right on the left
        let side = egui::FontId::proportional(11.0);
        painter.text(
            rect.left_top(),
            egui::Align2::LEFT_TOP,
            aurora_i18n::tr!("anim-picker-right"),
            side.clone(),
            palette.muted,
        );
        painter.text(
            rect.right_top(),
            egui::Align2::RIGHT_TOP,
            aurora_i18n::tr!("anim-picker-left"),
            side,
            palette.muted,
        );
        for (index, dot) in dots.iter().enumerate() {
            if let Some(parent) = dot.parent {
                painter.line_segment([points[parent], points[index]], egui::Stroke::new(1.5, palette.muted_dim));
            }
        }
        let at = self.clip.frame;
        let keyed = self
            .clip
            .timeline_tracks()
            .map_or_else(HashSet::new, |tracks| keyed_at(&tracks, at));
        // the selected and hovered dots are drawn last, over their neighbours
        let selected = dots.iter().position(|dot| self.selected == Some(dot.bone));
        let order = (0..dots.len())
            .filter(|index| Some(*index) != selected && Some(*index) != hovered)
            .chain(selected.filter(|index| Some(*index) != hovered))
            .chain(hovered);
        for index in order {
            let bone = dots[index].bone;
            let lit = Some(index) == hovered;
            let chosen = Some(index) == selected;
            let radius = if lit {
                6.0
            } else if chosen {
                5.0
            } else {
                3.5
            };
            if keyed.contains(&bone) {
                painter.circle_stroke(points[index], radius + 2.5, egui::Stroke::new(1.5, palette.violet_light));
            }
            let color = if chosen {
                palette.teal
            } else if lit {
                palette.ink
            } else {
                palette.muted
            };
            painter.circle_filled(points[index], radius, color);
        }
        let Some(index) = hovered else { return };
        let bone = dots[index].bone;
        let Some(name) = rig.canonical_names.get(bone) else { return };
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        response = response.on_hover_text_at_pointer(name);
        if response.clicked() {
            // a bone picked here shows its handles, whatever Display hides
            self.visibility.reveal(name);
            self.select(bone, rig, frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;

    fn figure(lib: &AvatarLibrary, view: View, rect: egui::Rect) -> (Vec<Dot>, Vec<egui::Pos2>) {
        let rig = &lib.rig;
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
        let dots = dots(rig, &rest, view);
        let points = fit(view, &dots, rect);
        (dots, points)
    }

    fn point_of(lib: &AvatarLibrary, dots: &[Dot], points: &[egui::Pos2], name: &str) -> egui::Pos2 {
        let bone = lib.rig.names[name];
        let index = dots
            .iter()
            .position(|dot| dot.bone == bone)
            .unwrap_or_else(|| panic!("{name} is drawn"));
        points[index]
    }

    #[test]
    fn body_holds_the_main_bones_and_leaves_the_extras_to_other_views() {
        for name in [
            "mPelvis",
            "mTorso",
            "mChest",
            "mNeck",
            "mHead",
            "mCollarLeft",
            "mShoulderRight",
            "mElbowLeft",
            "mWristRight",
            "mHipLeft",
            "mKneeRight",
            "mAnkleLeft",
            "mFootRight",
            "mToeLeft",
        ] {
            assert!(shows(View::Body, name), "{name}");
        }
        for name in [
            "mHandIndex1Left",
            "mHandThumb3Right",
            "mFaceJaw",
            "mEyeLeft",
            "mWingsRoot",
            "mTail1",
            "mHindLimb1Left",
            "mSpine1",
            "mSkull",
            "mGroin",
        ] {
            assert!(!shows(View::Body, name), "{name}");
        }
        assert!(shows(View::Hands, "mHandPinky2Left") && shows(View::Hands, "mWristRight"));
        assert!(!shows(View::Hands, "mElbowLeft") && !shows(View::Hands, "mFaceJaw"));
        assert!(shows(View::Face, "mFaceJaw") && shows(View::Face, "mEyeRight"));
        assert!(!shows(View::Face, "mHead") && !shows(View::Face, "mHandIndex1Left"));
    }

    #[test]
    fn every_view_draws_joints_of_the_avatar_rig() {
        let lib = AvatarLibrary::load();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(376.0, 225.0));
        for view in View::ALL {
            let (dots, points) = figure(&lib, view, rect);
            assert!(dots.len() > 8, "{view:?} has {} joints", dots.len());
            for point in &points {
                assert!(rect.expand(0.01).contains(*point), "{view:?} fits its area: {point:?}");
            }
            for dot in &dots {
                assert!(shows(view, &lib.rig.canonical_names[dot.bone]));
            }
        }
        let (body, _) = figure(&lib, View::Body, rect);
        let names: Vec<_> = body.iter().map(|dot| lib.rig.canonical_names[dot.bone].as_str()).collect();
        assert!(names.iter().all(|name| !name.starts_with("mHand")), "fingers are not in Body");
        assert_eq!(names.len(), 23, "{names:?}");
    }

    #[test]
    fn the_front_view_puts_the_avatar_s_left_on_the_right_and_its_head_on_top() {
        let lib = AvatarLibrary::load();
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(376.0, 225.0));
        let (dots, points) = figure(&lib, View::Body, rect);
        let at = |name| point_of(&lib, &dots, &points, name);
        assert!(at("mWristLeft").x > at("mWristRight").x);
        assert!(at("mShoulderLeft").x > at("mPelvis").x && at("mShoulderRight").x < at("mPelvis").x);
        assert!(at("mHead").y < at("mChest").y && at("mChest").y < at("mPelvis").y);
        assert!(at("mPelvis").y < at("mKneeLeft").y && at("mKneeLeft").y < at("mAnkleLeft").y);
        assert!(
            at("mToeLeft").distance(at("mFootLeft")) > 2.0,
            "the toe does not hide behind its foot"
        );
        assert!((at("mHead").x - at("mPelvis").x).abs() < 2.0, "the figure is symmetric");
        let (face, face_points) = figure(&lib, View::Face, rect);
        let eye = |name| point_of(&lib, &face, &face_points, name);
        assert!(eye("mEyeLeft").x > eye("mEyeRight").x);
    }

    #[test]
    fn each_hand_is_fitted_to_its_own_half() {
        let lib = AvatarLibrary::load();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(376.0, 225.0));
        let (dots, points) = figure(&lib, View::Hands, rect);
        assert!(dots.iter().any(|dot| dot.left) && dots.iter().any(|dot| !dot.left));
        for (dot, point) in dots.iter().zip(&points) {
            if dot.left {
                assert!(point.x >= rect.center().x, "{}", lib.rig.canonical_names[dot.bone]);
            } else {
                assert!(point.x <= rect.center().x, "{}", lib.rig.canonical_names[dot.bone]);
            }
        }
    }

    #[test]
    fn lines_join_each_joint_to_its_nearest_drawn_ancestor() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0));
        let parent_of = |view, name: &str| {
            let (dots, _) = figure(&lib, view, rect);
            let dot = dots.iter().find(|dot| dot.bone == rig.names[name]).expect("drawn");
            dot.parent.map(|parent| rig.canonical_names[dots[parent].bone].clone())
        };
        assert_eq!(parent_of(View::Body, "mElbowLeft").as_deref(), Some("mShoulderLeft"));
        assert_eq!(parent_of(View::Body, "mTorso").as_deref(), Some("mPelvis"), "past the spine pivots");
        assert_eq!(parent_of(View::Body, "mPelvis"), None);
        assert_eq!(parent_of(View::Hands, "mHandIndex1Right").as_deref(), Some("mWristRight"));
        assert_eq!(parent_of(View::Hands, "mHandIndex2Right").as_deref(), Some("mHandIndex1Right"));
    }

    #[test]
    fn only_the_user_s_keys_ring_a_dot() {
        let track = |bone, authored, keys: &[u32]| TimelineTrack {
            bone,
            name: String::new(),
            enabled: true,
            authored,
            keys: keys.to_vec(),
        };
        let tracks = [track(1, false, &[0]), track(2, true, &[0, 30]), track(3, true, &[15])];
        assert_eq!(keyed_at(&tracks, 0), HashSet::from([2]), "not the rest baseline of bone 1");
        assert_eq!(keyed_at(&tracks, 15), HashSet::from([3]));
        assert!(keyed_at(&tracks, 7).is_empty());
    }

    #[test]
    fn a_click_picks_the_nearest_dot_within_its_target() {
        let points = [egui::pos2(10.0, 10.0), egui::pos2(20.0, 10.0)];
        assert_eq!(hit(&points, egui::pos2(14.0, 11.0), HIT), Some(0));
        assert_eq!(hit(&points, egui::pos2(16.0, 9.0), HIT), Some(1));
        assert_eq!(hit(&points, egui::pos2(10.0, 30.0), HIT), None);
    }
}
