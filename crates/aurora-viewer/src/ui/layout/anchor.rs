//! Invisible resize anchors, chosen from corresponding perimeter points.
//! Unlike LLFloaterView::reshape (indra/llui/llfloater.cpp), the human's
//! requested placement retains one anchor and its signed offset until moved.

use egui::{Align2, Pos2, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Anchor {
    pub point: Align2,
    offset: Vec2,
}

impl Anchor {
    // Fixed order resolves exact ties consistently, including first launch.
    const POINTS: [Align2; 8] = [
        Align2::LEFT_TOP,
        Align2::RIGHT_TOP,
        Align2::LEFT_BOTTOM,
        Align2::RIGHT_BOTTOM,
        Align2::CENTER_TOP,
        Align2::RIGHT_CENTER,
        Align2::CENTER_BOTTOM,
        Align2::LEFT_CENTER,
    ];

    pub fn new(point: Align2, room: Rect, window: Rect) -> Self {
        Self {
            point,
            offset: point.pos_in_rect(&window) - point.pos_in_rect(&room),
        }
    }

    pub fn nearest(room: Rect, window: Rect) -> Self {
        let mut best = Self::new(Self::POINTS[0], room, window);
        for point in Self::POINTS.into_iter().skip(1) {
            let candidate = Self::new(point, room, window);
            if candidate.offset.length_sq() < best.offset.length_sq() {
                best = candidate;
            }
        }
        best
    }

    pub fn position(self, room: Rect, size: Vec2) -> Pos2 {
        let local = self.point.pos_in_rect(&Rect::from_min_size(Pos2::ZERO, size));
        self.point.pos_in_rect(&room) + self.offset - local.to_vec2()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corresponding_corners_and_side_midpoints_can_all_win() {
        let room = Rect::from_min_max(egui::pos2(40.0, 60.0), egui::pos2(1240.0, 860.0));
        let size = egui::vec2(200.0, 100.0);
        for point in Anchor::POINTS {
            let rect = point.anchor_size(point.pos_in_rect(&room), size);
            let anchor = Anchor::nearest(room, rect);
            assert_eq!(anchor.point, point);
            assert_eq!(anchor.offset, Vec2::ZERO);
            assert_eq!(anchor.position(room, size), rect.min);
        }
    }

    #[test]
    fn selection_compares_corresponding_points_instead_of_the_window_center() {
        let room = Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0));
        // Its center is closer to the top midpoint, but its top-right corner
        // is closer to the corresponding room corner than any other pair.
        let rect = Rect::from_min_size(egui::pos2(550.0, 30.0), egui::vec2(600.0, 100.0));
        let anchor = Anchor::nearest(room, rect);
        assert_eq!(anchor.point, Align2::RIGHT_TOP);
        assert_eq!(anchor.offset, egui::vec2(-50.0, 30.0));
    }

    #[test]
    fn stored_point_and_signed_offset_survive_resizing_and_a_shifted_origin() {
        let room = Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let size = egui::vec2(200.0, 100.0);
        let rect = Rect::from_min_size(egui::pos2(505.0, 20.0), size);
        let anchor = Anchor::nearest(room, rect);
        assert_eq!(anchor.point, Align2::CENTER_TOP);
        let changed = Rect::from_min_max(egui::pos2(40.0, 60.0), egui::pos2(1640.0, 1060.0));
        assert_eq!(anchor.position(changed, size), egui::pos2(745.0, 80.0));
        assert_eq!(anchor.position(room, size), rect.min);
        // An off-screen window retains the direction of its offset too.
        let outside = Rect::from_min_size(egui::pos2(1030.0, 720.0), size);
        let anchor = Anchor::nearest(room, outside);
        assert_eq!(anchor.point, Align2::RIGHT_BOTTOM);
        assert_eq!(anchor.offset, egui::vec2(30.0, 20.0));
    }
}
