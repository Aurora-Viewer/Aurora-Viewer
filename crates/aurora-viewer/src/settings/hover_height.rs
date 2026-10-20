//! Account binding for avatar height options. The posture heights themselves
//! stay in each account's world/hover_height.rs store, following
//! LLFloaterHoverHeight::syncFromPreferenceSetting (llfloaterhoverheight.cpp).
use super::Settings;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    automatic: bool,
    offset: f32,
    affect_camera: bool,
    ground_feet: bool,
    ground_while_moving: bool,
    ground_max_height: f32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: false,
            offset: 0.0,
            affect_camera: false,
            ground_feet: false,
            ground_while_moving: false,
            ground_max_height: super::DEFAULT_GROUND_FEET_HEIGHT,
        }
    }
}

impl Preferences {
    fn current(settings: &Settings) -> Self {
        Self {
            automatic: settings.auto_hover_height,
            offset: crate::world::hover_height::bounded(settings.auto_hover_offset).unwrap_or(0.0),
            affect_camera: settings.camera.hover_affects_camera,
            ground_feet: settings.ground_avatar_feet,
            ground_while_moving: settings.ground_feet_while_moving,
            ground_max_height: super::foot_height_limit(settings.ground_feet_max_height),
        }
    }

    fn apply(self, settings: &mut Settings) {
        settings.auto_hover_height = self.automatic;
        settings.auto_hover_offset = crate::world::hover_height::bounded(self.offset).unwrap_or(0.0);
        settings.camera.hover_affects_camera = self.affect_camera;
        settings.ground_avatar_feet = self.ground_feet;
        settings.ground_feet_while_moving = self.ground_while_moving;
        settings.ground_feet_max_height = super::foot_height_limit(self.ground_max_height);
    }
}

impl Settings {
    /// Bind before the first world update. Import the former shared values
    /// once; an account without saved options always starts from defaults.
    /// This deliberately extends the Legacy Viewer's per-account height to
    /// all Aurora height options, including HoverHeightAffectsCamera.
    pub(crate) fn select_hover_account(&mut self, account: Uuid) {
        self.remember_hover_account();
        let preferences = match &self.hover_height_accounts {
            None => Preferences::current(self),
            Some(accounts) => accounts.get(&account).copied().unwrap_or_default(),
        };
        self.hover_height_account = Some(account);
        preferences.apply(self);
        self.remember_hover_account();
    }

    pub(crate) fn release_hover_account(&mut self) {
        if self.hover_height_account.is_none() {
            return;
        }
        self.remember_hover_account();
        self.hover_height_account = None;
        Preferences::default().apply(self);
    }

    /// Every settings save captures changes from both Hover Height and
    /// Preferences, so neither UI path can overwrite another avatar's choice.
    pub(super) fn remember_hover_account(&mut self) {
        if let Some(account) = self.hover_height_account {
            let preferences = Preferences::current(self);
            self.hover_height_accounts
                .get_or_insert_with(Default::default)
                .insert(account, preferences);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restart(settings: &Settings) -> Settings {
        serde_json::from_slice(&serde_json::to_vec(&settings.to_saved()).unwrap()).unwrap()
    }

    #[test]
    fn hover_accounts_keep_all_options_on_switch_save_and_restart() {
        let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut settings = Settings::default();
        settings.select_hover_account(first);
        settings.auto_hover_height = true;
        settings.auto_hover_offset = -0.05;
        settings.camera.hover_affects_camera = true;
        settings.ground_avatar_feet = true;
        settings.ground_feet_while_moving = true;
        settings.ground_feet_max_height = 0.8;
        let first_options = Preferences::current(&settings);
        settings.select_hover_account(second);
        assert_eq!(Preferences::current(&settings), Preferences::default());
        settings.auto_hover_offset = 0.12;
        settings.ground_feet_max_height = 0.2;
        let second_options = Preferences::current(&settings);
        settings.select_hover_account(first);
        assert_eq!(Preferences::current(&settings), first_options);
        settings.select_hover_account(second);
        assert_eq!(Preferences::current(&settings), second_options);
        let mut loaded = restart(&settings);
        assert_eq!(loaded.hover_height_account, None);
        // Unrelated saves from the login screen retain both accounts.
        loaded.vsync = true;
        loaded = restart(&loaded);
        loaded.select_hover_account(first);
        assert_eq!(Preferences::current(&loaded), first_options);
        loaded.select_hover_account(second);
        assert_eq!(Preferences::current(&loaded), second_options);
    }

    #[test]
    fn hover_accounts_save_the_active_options_without_a_switch() {
        let account = Uuid::from_u128(1);
        let mut settings = Settings::default();
        settings.select_hover_account(account);
        settings.auto_hover_height = true;
        settings.auto_hover_offset = -0.17;
        settings.camera.hover_affects_camera = true;
        let mut loaded = restart(&settings);
        loaded.select_hover_account(account);
        assert_eq!(Preferences::current(&loaded), Preferences::current(&settings));
    }

    #[test]
    fn hover_accounts_logout_stops_login_screen_edits_from_changing_the_last_account() {
        let account = Uuid::from_u128(1);
        let mut settings = Settings::default();
        settings.select_hover_account(account);
        settings.auto_hover_height = true;
        settings.auto_hover_offset = -0.17;
        let saved = Preferences::current(&settings);
        settings.release_hover_account();
        assert_eq!(Preferences::current(&settings), Preferences::default());
        settings.auto_hover_offset = 0.5;
        let mut loaded = restart(&settings);
        loaded.select_hover_account(account);
        assert_eq!(Preferences::current(&loaded), saved);
    }

    #[test]
    fn hover_accounts_import_old_shared_options_only_for_the_first_account() {
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "auto_hover_height": true, "auto_hover_offset": -0.09,
            "camera": {"hover_affects_camera": true},
            "ground_avatar_feet": true, "ground_feet_while_moving": true, "ground_feet_max_height": 0.7
        }))
        .unwrap();
        let former = Preferences::current(&settings);
        // Canceling a login before entering the world must not consume it.
        settings.release_hover_account();
        assert_eq!(Preferences::current(&settings), former);
        // A pre-login preferences save must not lose the migration.
        settings = restart(&settings);
        settings.select_hover_account(Uuid::from_u128(1));
        assert_eq!(Preferences::current(&settings), former);
        settings = restart(&settings);
        settings.select_hover_account(Uuid::from_u128(2));
        assert_eq!(Preferences::current(&settings), Preferences::default());
        settings.select_hover_account(Uuid::from_u128(1));
        assert_eq!(Preferences::current(&settings), former);
    }

    #[test]
    fn hover_accounts_reset_only_the_active_account() {
        let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut settings = Settings::default();
        settings.select_hover_account(first);
        settings.auto_hover_height = true;
        settings.auto_hover_offset = 0.25;
        let first_options = Preferences::current(&settings);
        settings.select_hover_account(second);
        settings.auto_hover_height = true;
        settings.auto_hover_offset = -0.1;
        settings.reset_keeping_account();
        let mut loaded = restart(&settings);
        loaded.select_hover_account(second);
        assert_eq!(Preferences::current(&loaded), Preferences::default());
        loaded.select_hover_account(first);
        assert_eq!(Preferences::current(&loaded), first_options);
        loaded = restart(&loaded);
        loaded.reset_keeping_account();
        loaded.select_hover_account(first);
        assert_eq!(Preferences::current(&loaded), first_options);
    }

    #[test]
    fn hover_accounts_bound_saved_numeric_options_on_selection() {
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "hover_height_accounts": {
                "00000000-0000-0000-0000-000000000001": {"offset": 12.0, "ground_max_height": -2.0}
            }
        }))
        .unwrap();
        settings.select_hover_account(Uuid::from_u128(1));
        assert_eq!(settings.auto_hover_offset, crate::world::hover_height::MAX);
        assert_eq!(settings.ground_feet_max_height, 0.0);
    }
}
