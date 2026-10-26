//! Experimental remote foot-placement budget. Normal operation remains unlimited.
//! Reuse the renderer's main frustum; distance is from our avatar, not the camera.
use super::CullView;
use glam::Vec3;
use std::time::{Duration, Instant};
use uuid::Uuid;

const LIMIT: usize = 10;

#[derive(Default)]
pub(super) struct Scope {
    limited: bool,
    nearest: Vec<(Uuid, f32)>,
    pub support_frames: usize,
    profile: bool,
    logged: Option<Instant>,
}

impl Scope {
    pub fn from_environment() -> Self {
        Self {
            profile: std::env::var_os("AURORA_PROFILE").is_some(),
            ..Self::from_mode(&std::env::var("AURORA_FOOT_IK_SCOPE").unwrap_or_default())
        }
    }

    pub fn from_mode(mode: &str) -> Self {
        Self {
            limited: mode == "visible10",
            ..Default::default()
        }
    }

    pub fn allows(&self, avatar: Uuid) -> bool {
        !self.limited || self.nearest.iter().any(|&(id, _)| id == avatar)
    }

    pub fn select(&mut self, candidates: impl Iterator<Item = (Uuid, Vec3, f32)>, origin: Vec3, view: CullView) {
        self.nearest.clear();
        if !self.limited {
            return;
        }
        for (avatar, position, radius) in candidates {
            let distance = position.distance_squared(origin);
            if !position.is_finite() || !distance.is_finite() || !radius.is_finite() || !view.sphere_visible(position, radius) {
                continue;
            }
            let key = (distance, avatar);
            let at = self
                .nearest
                .partition_point(|&(id, old)| old.total_cmp(&key.0).then_with(|| id.cmp(&key.1)).is_lt());
            if at < LIMIT {
                self.nearest.insert(at, (avatar, distance));
                self.nearest.truncate(LIMIT);
            }
        }
    }

    pub fn clear(&mut self) {
        self.nearest.clear();
        self.support_frames = 0;
        self.logged = None;
    }

    pub fn log(&mut self, enabled: bool, cached: usize, now: Instant) {
        if self.profile
            && self
                .logged
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        {
            self.logged = Some(now);
            let mode = if !enabled {
                "off"
            } else if self.limited {
                "visible10"
            } else {
                "all"
            };
            let selected = if !enabled {
                0
            } else if self.limited {
                self.nearest.len()
            } else {
                cached
            };
            log::info!(target: "aurora_viewer::foot_scope",
                "foot scope: mode={mode} selected_remote={selected} cached_remote={cached} support_frames={}", self.support_frames);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> CullView {
        let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
        CullView::new(
            projection * glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z),
            Vec3::ZERO,
            900.0,
            std::f32::consts::FRAC_PI_2,
        )
    }

    #[test]
    fn visible_ten_budget_uses_avatar_distance_and_rejects_behind_camera() {
        let mut scope = Scope {
            limited: true,
            ..Default::default()
        };
        let origin = Vec3::X * 30.0;
        let candidates = (1..=30)
            .map(|n| (Uuid::from_u128(n), Vec3::X * n as f32, 0.1))
            .chain([(Uuid::nil(), -Vec3::X, 0.1)]);
        scope.select(candidates, origin, view());
        assert_eq!(scope.nearest.len(), LIMIT);
        for n in 1..=30 {
            assert_eq!(scope.allows(Uuid::from_u128(n)), n >= 21, "distance from our avatar, not camera");
        }
        assert!(!scope.allows(Uuid::nil()));
    }

    #[test]
    fn visible_ten_ties_are_stable_and_camera_turn_replaces_the_budget() {
        let mut scope = Scope {
            limited: true,
            ..Default::default()
        };
        for backwards in [false, true] {
            let mut ids: Vec<_> = (1..=20).collect();
            if backwards {
                ids.reverse();
            }
            scope.select(
                ids.into_iter().map(|n| (Uuid::from_u128(n), Vec3::X * 3.0, 0.1)),
                Vec3::ZERO,
                view(),
            );
            assert_eq!(
                scope.nearest.iter().map(|x| x.0.as_u128()).collect::<Vec<_>>(),
                (1..=10).collect::<Vec<_>>()
            );
        }
        let turned = CullView::new(
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.0, 0.1)
                * glam::camera::rh::view::look_at_mat4(Vec3::ZERO, -Vec3::X, Vec3::Z),
            Vec3::ZERO,
            900.0,
            1.0,
        );
        scope.select([(Uuid::from_u128(40), -Vec3::X * 3.0, 0.1)].into_iter(), Vec3::ZERO, turned);
        assert_eq!(scope.nearest.len(), 1);
        assert!(scope.allows(Uuid::from_u128(40)));
        assert!(!scope.allows(Uuid::from_u128(1)));
        assert!(Scope::default().allows(Uuid::nil()), "normal operation remains unlimited");
    }
}
