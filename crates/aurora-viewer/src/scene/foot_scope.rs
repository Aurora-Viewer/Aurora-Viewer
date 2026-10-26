//! Saved remote foot-placement budget; our avatar stays outside the limit.
//! Reuse the renderer's main frustum; distance is from our avatar, not the camera.
use super::CullView;
use glam::Vec3;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Default)]
enum Experiment {
    #[default]
    Saved,
    All,
    Ten,
}

pub(super) struct Scope {
    limit: Option<usize>,
    experiment: Experiment,
    nearest: Vec<(Uuid, f32)>,
    pub support_frames: usize,
    profile: bool,
    logged: Option<Instant>,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            limit: Some(crate::settings::DEFAULT_GROUND_FEET_AVATARS as usize),
            experiment: Experiment::Saved,
            nearest: Vec::new(),
            support_frames: 0,
            profile: false,
            logged: None,
        }
    }
}

impl Scope {
    pub fn from_environment() -> Self {
        // Keep historical A/B controls in offline demos only. A normal viewer
        // always follows the saved preference, including existing accounts.
        let mode = std::env::var("AURORA_FOOT_IK_SCOPE")
            .ok()
            .filter(|_| std::env::var_os("AURORA_DEMO").is_some());
        Self {
            profile: std::env::var_os("AURORA_PROFILE").is_some(),
            ..Self::from_mode(mode.as_deref().unwrap_or_default())
        }
    }

    pub fn from_mode(mode: &str) -> Self {
        Self {
            limit: match mode {
                "all" => None,
                "visible10" => Some(10),
                _ => Some(crate::settings::DEFAULT_GROUND_FEET_AVATARS as usize),
            },
            experiment: match mode {
                "all" => Experiment::All,
                "visible10" => Experiment::Ten,
                _ => Experiment::Saved,
            },
            ..Default::default()
        }
    }

    pub fn configure(&mut self, maximum: u32) {
        self.limit = match self.experiment {
            Experiment::Saved => Some(maximum.min(crate::settings::MAX_GROUND_FEET_AVATARS) as usize),
            Experiment::All => None,
            Experiment::Ten => Some(10),
        };
    }

    pub fn allows(&self, avatar: Uuid) -> bool {
        self.limit.is_none() || self.nearest.iter().any(|&(id, _)| id == avatar)
    }

    pub fn select(&mut self, candidates: impl Iterator<Item = (Uuid, Vec3, f32)>, origin: Vec3, view: CullView) {
        self.nearest.clear();
        let Some(limit) = self.limit.filter(|&limit| limit > 0) else {
            return;
        };
        for (avatar, position, radius) in candidates {
            let distance = position.distance_squared(origin);
            if !position.is_finite() || !distance.is_finite() || !radius.is_finite() || !view.sphere_visible(position, radius) {
                continue;
            }
            let key = (distance, avatar);
            let at = self
                .nearest
                .partition_point(|&(id, old)| old.total_cmp(&key.0).then_with(|| id.cmp(&key.1)).is_lt());
            if at < limit {
                self.nearest.insert(at, (avatar, distance));
                self.nearest.truncate(limit);
            }
        }
    }

    pub fn clear_selection(&mut self) {
        self.nearest.clear();
    }

    pub fn clear(&mut self) {
        self.clear_selection();
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
            } else if self.limit.is_some() {
                "visible"
            } else {
                "all"
            };
            let selected = if !enabled {
                0
            } else if self.limit.is_some() {
                self.nearest.len()
            } else {
                cached
            };
            let limit = self.limit.map_or_else(|| "all".to_owned(), |limit| limit.to_string());
            log::info!(target: "aurora_viewer::foot_scope",
                "foot scope: mode={mode} limit={limit} selected_remote={selected} cached_remote={cached} support_frames={}", self.support_frames);
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
        let mut scope = Scope::from_mode("visible10");
        let origin = Vec3::X * 30.0;
        let candidates = (1..=30)
            .map(|n| (Uuid::from_u128(n), Vec3::X * n as f32, 0.1))
            .chain([(Uuid::nil(), -Vec3::X, 0.1)]);
        scope.select(candidates, origin, view());
        assert_eq!(scope.nearest.len(), 10);
        for n in 1..=30 {
            assert_eq!(scope.allows(Uuid::from_u128(n)), n >= 21, "distance from our avatar, not camera");
        }
        assert!(!scope.allows(Uuid::nil()));
    }

    #[test]
    fn visible_ten_ties_are_stable_and_camera_turn_replaces_the_budget() {
        let mut scope = Scope::from_mode("visible10");
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
        assert!(Scope::from_mode("all").allows(Uuid::nil()), "historical demo baseline");
    }

    #[test]
    fn saved_avatar_budget_defaults_to_five_and_updates_live_including_zero() {
        let mut scope = Scope::default();
        for maximum in [5, 1, 0, 12, 100, u32::MAX] {
            scope.configure(maximum);
            scope.select((1..=120).map(|n| (Uuid::from_u128(n), Vec3::X * n as f32, 0.1)), Vec3::ZERO, view());
            let expected = maximum.min(crate::settings::MAX_GROUND_FEET_AVATARS) as usize;
            assert_eq!(scope.nearest.len(), expected);
            assert_eq!(scope.nearest.iter().filter(|&&(id, _)| scope.allows(id)).count(), expected);
        }
        let mut default = Scope::default();
        default.select((1..=20).map(|n| (Uuid::from_u128(n), Vec3::X * n as f32, 0.1)), Vec3::ZERO, view());
        assert_eq!(default.nearest.len(), 5);
    }
}
