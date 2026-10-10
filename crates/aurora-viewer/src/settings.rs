//! Persistent user settings (JSON in the platform config directory).
//! Passwords are never stored.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GridChoice {
    SecondLife,
    SecondLifeBeta,
    Custom,
}

/// Debug overlays (Préférences › Debug), not saved: they reset at each launch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DebugView {
    /// Complexity under the name tags, green to red (Firestorm FSTagShowARW).
    pub complexity_tags: bool,
    /// « Afficher la transparence » (Ctrl+Alt+T, LLDrawPoolAlpha::sShowDebugAlpha).
    pub show_alpha: bool,
    /// ... rigged faces too (Firestorm sShowDebugAlphaRigged).
    pub show_alpha_rigged: bool,
    pub wireframe: bool,
    /// Object bounding boxes.
    pub bounds: bool,
    /// Bounding boxes colored by what the culling decided.
    pub culling: bool,
    /// Keep the culling view where it is (move around to see what it drops).
    pub freeze_culling: bool,
    pub lights: bool,
    /// Objects with glowing faces.
    pub glow_objects: bool,
    /// Glow amount view (red = glow x10).
    pub glow_view: bool,
    pub probes: bool,
    pub skeletons: bool,
}

impl DebugView {
    /// An overlay drawn over the 3D view is on.
    pub fn any_overlay(&self) -> bool {
        self.bounds || self.culling || self.lights || self.glow_objects || self.probes || self.skeletons
    }
}

/// Look-at range slider maximum: no limit.
pub const LOOK_AT_UNLIMITED: f32 = 64.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// ShowHUDAttachments, enabled by default in Firestorm.
    pub show_huds: bool,
    /// Debug overlays (not saved).
    #[serde(skip)]
    pub debug: DebugView,
    /// Build tools (grid, snap, create, land brush).
    pub build: crate::build::BuildSettings,
    /// Camera (Firestorm « Déplacements et vue › Vue »).
    pub camera: crate::camera::CameraSettings,
    /// Contacts window (Amis / Groupes / Cercles).
    pub contacts: crate::ui::contacts::ContactsSettings,
    pub inventory: crate::ui::inventory::InventoryPreferences,
    pub username: String,
    pub remember_username: bool,
    /// Remember the password (its login hash, in the system credential
    /// store; Firestorm "Remember password"). Off by default.
    pub remember_password: bool,
    pub grid: GridChoice,
    pub custom_login_uri: String,
    pub start_location: String,
    pub start_region: String,
    pub draw_distance: f32,
    pub vsync: bool,
    pub shadows: bool,
    pub force_midday: bool,
    /// Local time of day: 0 shared (region EEP), 1 sunrise, 2 noon, 3 sunset, 4 midnight.
    pub time_of_day: u8,
    /// Crossfade (s) of manual environment changes: selector, presets
    /// (Firestorm FSEnvironmentManualTransitionTime, 0 = instant).
    pub env_manual_transition: f32,
    /// Keep the local environment from one session to the next (Firestorm
    /// EnvironmentPersistAcrossLogin, on).
    pub env_persist: bool,
    pub exposure: f32,
    /// Contrast adaptive sharpening (Firestorm RenderCASSharpness, 0.4).
    pub sharpen: f32,
    /// Glow of faces with a glow value (Firestorm RenderGlow, on).
    pub glow: bool,
    /// GPU occlusion culling: objects hidden behind others are not drawn
    /// (Firestorm UseOcclusion, on by default). Renamed from `gpu_occlusion`
    /// (off while it cost more than it saved) so that saved settings take
    /// the new default.
    pub occlusion_culling: bool,
    pub lod_factor: f32,
    /// Texture memory sized from the detected video memory (else `texture_budget_mb`).
    pub texture_budget_auto: bool,
    pub texture_budget_mb: u32,
    pub ui_scale: f32,
    pub show_perf: bool,
    pub show_chat: bool,
    pub show_minimap: bool,
    /// FSUseStandalonePlaceDetailsFloater: landmarks, place links and history
    /// entries open in their own window instead of the Places window.
    pub standalone_place_details: bool,
    /// LandmarksSortedByDate: the Landmarks tab of Places sorted by date.
    pub landmarks_by_date: bool,
    /// Mini-map and world map options (Firestorm MiniMap* / Map* settings).
    pub maps: MapSettings,
    /// Other panels open at exit (restored at startup) and the People tab.
    pub show_people: bool,
    pub people_tab: u8,
    pub show_inventory: bool,
    pub show_appearance: bool,
    /// Loading / teleport screens show the last view, blurred.
    pub loading_backdrop: bool,
    pub mouse_sensitivity: f32,
    pub window_width: u32,
    pub window_height: u32,
    /// Last window position (outer, physical pixels) and maximized state
    /// (Firestorm WindowX / WindowY / WindowMaximized).
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub window_maximized: bool,
    /// Random stable machine identifier (hashed before being sent).
    pub machine_id: String,
    pub mfa_hash: String,
    pub font: String,
    pub font_scale: f32,
    pub chat_timestamps: bool,
    pub chat_toast_seconds: f32,
    /// Ask before opening a web link outside the trusted domains (the box
    /// « Ne plus me prévenir » of the warning turns it off).
    pub warn_external_links: bool,
    /// FSMuteAllGroups: refuse every group chat.
    pub mute_all_groups: bool,
    /// FSMuteGroupWhenNoticesDisabled: refuse group chat of groups whose
    /// notices are turned off.
    pub mute_groups_without_notices: bool,
    /// FSReportBlockToNearbyChat: say in nearby chat when the block list changes.
    pub report_blocks: bool,
    /// Automatic responses and the kept online status modes.
    pub autoresponse: crate::world::status::AutoResponse,
    /// The chat bar as a command line (FSCmdLine*, Préférences › Chat ›
    /// Commandes).
    pub chat_commands: crate::cmdline::ChatCommandSettings,
    pub name_tag_distance: f32,
    pub invert_mouse: bool,
    pub arrows_strafe: bool,
    /// Our avatar's head and eyes follow the cursor and what we point at,
    /// and others see where we look (LookAt). Off by default (Firestorm:
    /// DisableLookAtAnimation / PrivateLookAtTarget).
    pub look_at: bool,
    /// Look-at range (m): a farther target is brought back to this distance
    /// from the head before being shown (FSLookAtTargetMaxDistance);
    /// LOOK_AT_UNLIMITED = no limit.
    pub look_at_range: f32,
    /// 0 off, 1 low, 2 medium, 3 high, 4 ultra
    pub shadow_quality: u8,
    /// 0 none, 1 FXAA, 2 MSAA 2x, 3 MSAA 4x, 4 MSAA 8x, 5 TAA
    pub antialiasing: u8,
    /// 0 Khronos PBR Neutral, 1 ACES
    pub tonemapper: u8,
    /// Frame rate cap (off by default) and its value (images per second).
    pub fps_cap: bool,
    pub fps_limit: u32,
    /// Lower cap while the window is not focused (on by default, as
    /// Firestorm yields BackgroundYieldTime every frame in the background).
    pub background_fps_cap: bool,
    pub background_fps_limit: u32,
    /// Maximum live particles (0 = particles off).
    pub max_particles: u32,
    /// Avatars drawn (the nearest ones).
    pub max_avatars: u32,
    /// Ambient occlusion: 0 off, 1 low, 2 high.
    pub ssao: u8,
    /// Water reflections: 0 off, 1 low, 2 medium, 3 high, 4 ultra.
    pub water_reflections: u8,
    /// Mirrors: 0 off, 1 low, 2 medium, 3 high, 4 ultra.
    pub mirrors: u8,
    /// Reflection probes (RenderReflectionProbeLevel): 0 none (sky only),
    /// 1 manual, 2 manual + terrain, 3 full scene.
    pub reflection_probes: u8,
    /// Reflection probe cube slots (RenderReflectionProbeCount; 8..=64).
    pub probe_count: u32,
    /// RenderAvatarMaxComplexity: avatars above it show as grey silhouettes
    /// (0 = no limit; Firestorm's Ultra preset is 350 000).
    pub max_complexity: u32,
    /// RenderAutoMuteSurfaceAreaLimit: attachments' surface area (m²) above
    /// which an avatar also shows as a silhouette (0 = no limit; ignored
    /// without a complexity limit, as in LL).
    pub max_attachment_area: f32,
    /// Avatars still loading: 0 particle cloud + progress bar under the name
    /// (Firestorm), 1 progress bar only (drawn as they load), 2 nothing.
    pub loading_avatars: u8,
    /// RenderAvatarComplexityMode: 0 limit everyone, 1 always show friends
    /// fully, 2 show only friends.
    pub complexity_mode: u8,
    /// Per-avatar exceptions (LLRenderMuteList): avatar id -> 1 never render
    /// fully (silhouette), 2 always render fully.
    pub render_exceptions: Vec<(String, u8)>,
    /// Complexity in the name tags (FSTagShowARW, on), only for avatars over
    /// the limit (FSTagShowTooComplexOnlyARW, on), ours too (FSTagShowOwnARW, off).
    pub tag_complexity: bool,
    pub tag_complexity_too_complex_only: bool,
    pub tag_complexity_own: bool,
    /// Display names (UseDisplayNames, on), the username with them
    /// (NameTagShowUsernames, on) as "Firstname Lastname"
    /// (FSNameTagShowLegacyUsernames, on), "Resident" hidden
    /// (FSTrimLegacyNames, on), name changes told in the chat
    /// (FSShowDisplayNameUpdateNotification, on).
    pub use_display_names: bool,
    pub show_usernames: bool,
    pub legacy_username_format: bool,
    pub trim_resident: bool,
    pub display_name_notices: bool,
    /// Screen-space reflections.
    pub ssr: bool,
    /// Anisotropic filtering: 1 (off), 2, 4, 8, 16.
    pub anisotropy: u16,
    /// Disk cache size (MB) for textures, meshes, animations, objects and inventory.
    pub cache_size_mb: u32,
    /// Width of the local chat input in the bottom bar (px, 0 = skin default).
    pub chat_bar_width: f32,
    /// Volumes and toggles (Firestorm volume panel).
    pub audio: AudioSettings,
    /// Parcel media and media on a prim.
    pub media: crate::media::MediaSettings,
    /// Name tag, minimap and chat colors (Firestorm color preferences).
    pub colors: ColorSettings,
    /// Rebindable keyboard / mouse controls.
    pub keybinds: crate::keybinds::KeyBindings,
    /// Préférences › Pratique: hold this key while dragging visible HUDs.
    /// Aurora shortcut requested by the user; ALT by default.
    pub hud_drag_key: crate::keybinds::HoldKey,
    /// Bumped when defaults change in a way that must reach existing settings
    /// (missing in older files: 0).
    #[serde(default)]
    pub settings_version: u32,
}

const SETTINGS_VERSION: u32 = 9;

/// RGBA color (sRGB, straight alpha).
pub type Rgba = [u8; 4];

const fn hex(v: u32) -> Rgba {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
}

/// Firestorm color preferences (colors.xml names in comments); defaults
/// follow the Aurora skin palette.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ColorSettings {
    // name tags
    pub tag_me: Rgba,
    pub tag_friend: Rgba,
    pub tag_muted: Rgba,
    pub tag_linden: Rgba,
    /// Display name equals the username (NameTagMatch).
    pub tag_match: Rgba,
    /// Custom display name (NameTagMismatch).
    pub tag_mismatch: Rgba,
    // minimap
    pub map_me: Rgba,
    pub map_other: Rgba,
    pub map_friend: Rgba,
    pub map_linden: Rgba,
    pub map_muted: Rgba,
    pub map_ranges: bool,
    pub map_whisper_on: bool,
    pub map_say_on: bool,
    pub map_shout_on: bool,
    pub map_whisper: Rgba,
    pub map_say: Rgba,
    pub map_shout: Rgba,
    // chat messages
    pub chat_mine: Rgba,
    pub chat_others: Rgba,
    pub chat_objects: Rgba,
    pub chat_friends: Rgba,
    pub chat_linden: Rgba,
    pub chat_muted: Rgba,
    pub chat_system: Rgba,
    /// Script errors (DEBUG_CHANNEL).
    pub chat_errors: Rgba,
    pub chat_object_im: Rgba,
    /// llOwnerSay.
    pub chat_owner: Rgba,
    pub chat_urls: Rgba,
    /// secondlife:/// links.
    pub chat_slurl: Rgba,
    /// llRegionSayTo.
    pub chat_direct: Rgba,
    // sender names
    pub sender_avatar: Rgba,
    pub sender_object: Rgba,
    // chat mentions
    pub mention_text: Rgba,
    pub mention_residents: Rgba,
    pub mention_me: Rgba,
}

impl Default for ColorSettings {
    fn default() -> Self {
        ColorSettings {
            tag_me: hex(0xFFFFFF),
            tag_friend: hex(0x818CF8),
            tag_muted: hex(0x6F7BA0),
            tag_linden: hex(0xF472B6),
            tag_match: hex(0xE8EDFB),
            tag_mismatch: hex(0xE8EDFB),
            map_me: hex(0x8B5CF6),
            map_other: hex(0x4ADE80),
            map_friend: hex(0x818CF8),
            map_linden: hex(0xF472B6),
            map_muted: hex(0x6F7BA0),
            map_ranges: true,
            map_whisper_on: true,
            map_say_on: true,
            map_shout_on: true,
            map_whisper: hex(0x6CC24A),
            map_say: hex(0x6CC24A),
            map_shout: hex(0xE0661A),
            chat_mine: hex(0x9AA6C6),
            chat_others: hex(0xE8EDFB),
            chat_objects: hex(0x4ADE80),
            chat_friends: hex(0x818CF8),
            chat_linden: hex(0xF472B6),
            chat_muted: hex(0x6F7BA0),
            chat_system: hex(0x9AA6C6),
            chat_errors: hex(0xF87171),
            chat_object_im: hex(0xA78BFA),
            chat_owner: hex(0xFCD34D),
            chat_urls: hex(0x5EEAD4),
            // every clickable link of a text (web, place, avatar, group)
            // shares the brand teal
            chat_slurl: hex(0x5EEAD4),
            chat_direct: hex(0xFB923C),
            sender_avatar: hex(0xC4B5FD),
            sender_object: hex(0x4ADE80),
            mention_text: hex(0xFCD34D),
            mention_residents: hex(0x8B5CF6),
            mention_me: hex(0x4F46E5),
        }
    }
}

/// Audio channels in Firestorm's order: Principal, Interface, Ambiance, Sons,
/// Musique, Multimédia, Voix.
pub const AUDIO_CHANNELS: [&str; 7] = ["Principal", "Interface", "Ambiance", "Sons", "Musique", "Multimédia", "Voix"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioSettings {
    /// Volume 0..1 per channel (`AUDIO_CHANNELS`).
    pub volume: [f32; 7],
    pub muted: [bool; 7],
    /// Channel enabled (checkbox; the master has none).
    pub enabled: [bool; 7],
    /// Play the parcel music automatically when entering a parcel
    /// (FSParcelMusicAutoPlay, off by default: the radio toggle starts it).
    pub music_autoplay: bool,
    /// Play parcel media automatically.
    pub media_autoplay: bool,
    /// Output device name (empty = system default).
    pub output_device: String,
    /// Microphone name (empty = system default).
    pub input_device: String,
    /// Connect to voice chat.
    pub voice_enabled: bool,
    /// Microphone button of the bottom bar: false = toggle (click on / click
    /// off), true = hold to talk (Firestorm PushToTalkToggle inverted).
    pub mic_hold: bool,
    /// Microphone gain (0..2).
    pub mic_gain: f32,
    /// Collision sounds (EnableCollisionSounds, on).
    pub collision_sounds: bool,
    /// Sounds of other avatars' gestures (EnableGestureSounds, on).
    pub gesture_sounds: bool,
    /// Interface sounds (IM, money, teleport, windows, menus, typing...;
    /// Firestorm's PlayModeUISnd* defaults).
    pub ui_sounds: bool,
    /// Each interface sound: played or not, asset (UISnd* / PlayModeUISnd*).
    pub ui: crate::ui_sound::UiSoundSettings,
    /// Saved music stream URLs (FSStreamList "audio", FIRE-593), offered by
    /// the "Son" tab of About Land.
    pub saved_streams: Vec<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings {
            volume: [0.8, 0.5, 0.5, 0.5, 0.4, 0.5, 0.7],
            muted: [false; 7],
            enabled: [true; 7],
            music_autoplay: false,
            media_autoplay: false,
            output_device: String::new(),
            input_device: String::new(),
            voice_enabled: true,
            mic_hold: false,
            mic_gain: 1.0,
            collision_sounds: true,
            gesture_sounds: true,
            ui_sounds: true,
            ui: Default::default(),
            saved_streams: Vec::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_huds: true,
            debug: DebugView::default(),
            build: Default::default(),
            camera: Default::default(),
            contacts: Default::default(),
            inventory: Default::default(),
            username: String::new(),
            remember_username: true,
            remember_password: false,
            grid: GridChoice::SecondLife,
            custom_login_uri: "http://127.0.0.1:9000/".into(),
            start_location: "last".into(),
            start_region: String::new(),
            draw_distance: 96.0,
            vsync: false,
            shadows: true,
            force_midday: false,
            time_of_day: 0,
            env_manual_transition: 0.0,
            env_persist: true,
            exposure: 1.0,
            sharpen: 0.4,
            glow: true,
            occlusion_culling: true,
            lod_factor: 2.0,
            texture_budget_auto: true,
            texture_budget_mb: 1536,
            ui_scale: 1.0,
            show_perf: true,
            show_chat: true,
            show_minimap: true,
            standalone_place_details: false,
            landmarks_by_date: true,
            maps: MapSettings::default(),
            show_people: false,
            people_tab: 0,
            show_inventory: false,
            show_appearance: false,
            loading_backdrop: true,
            mouse_sensitivity: 1.0,
            window_width: 1600,
            window_height: 900,
            window_x: None,
            window_y: None,
            window_maximized: false,
            machine_id: uuid::Uuid::new_v4().to_string(),
            mfa_hash: String::new(),
            font: "Noto Sans".into(),
            font_scale: 1.0,
            chat_timestamps: true,
            chat_toast_seconds: 20.0,
            warn_external_links: true,
            mute_all_groups: false,
            mute_groups_without_notices: false,
            report_blocks: false,
            autoresponse: Default::default(),
            chat_commands: Default::default(),
            name_tag_distance: 64.0,
            invert_mouse: false,
            arrows_strafe: false,
            look_at: false,
            look_at_range: 10.0,
            shadow_quality: 3,
            antialiasing: 3,
            tonemapper: 0,
            fps_cap: false,
            fps_limit: 60,
            background_fps_cap: true,
            background_fps_limit: 15,
            max_particles: 4096,
            max_avatars: 16,
            ssao: 1,
            water_reflections: 1,
            reflection_probes: 3,
            probe_count: 32,
            max_complexity: 350_000,
            max_attachment_area: 1000.0,
            loading_avatars: 0,
            complexity_mode: 0,
            render_exceptions: Vec::new(),
            tag_complexity: true,
            tag_complexity_too_complex_only: true,
            tag_complexity_own: false,
            use_display_names: true,
            show_usernames: true,
            legacy_username_format: true,
            trim_resident: true,
            display_name_notices: true,
            mirrors: 1,
            ssr: false,
            anisotropy: 8,
            cache_size_mb: 20480,
            chat_bar_width: 0.0,
            audio: AudioSettings::default(),
            media: Default::default(),
            colors: ColorSettings::default(),
            keybinds: Default::default(),
            hud_drag_key: Default::default(),
            settings_version: SETTINGS_VERSION,
        }
    }
}

/// Automatic texture memory (MB) from the detected video memory: 70 % of
/// it, at least 1.5 GB left to the rest, never under 768 MB. Integrated
/// GPUs share the system memory: 2 GB; unknown: 1.5 GB.
pub fn auto_texture_budget(vram_mb: Option<u64>, integrated: bool) -> u64 {
    match vram_mb {
        Some(v) if !integrated && v >= 1024 => (v * 7 / 10).min(v.saturating_sub(1536)).max(768),
        _ if integrated => 2048,
        _ => 1536,
    }
}

/// The offline demo keeps its own settings and cache (a `demo` folder inside
/// each), so test runs never change the user's real ones.
fn for_mode(dir: PathBuf) -> PathBuf {
    if std::env::var_os("AURORA_DEMO").is_some() {
        dir.join("demo")
    } else {
        dir
    }
}

pub fn config_dir() -> PathBuf {
    for_mode(
        directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    )
}

/// Files of one account (LL_PATH_PER_SL_ACCOUNT): contact sets, pinned
/// groups.
pub fn account_dir(agent: &uuid::Uuid) -> PathBuf {
    config_dir().join("accounts").join(agent.to_string())
}

pub fn cache_dir() -> PathBuf {
    for_mode(
        directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("cache")),
    )
}

impl Settings {
    /// Rendering exception of an avatar: 0 none, 1 never render fully,
    /// 2 always render fully.
    pub fn render_exception(&self, id: &uuid::Uuid) -> u8 {
        let s = id.to_string();
        self.render_exceptions.iter().find(|(x, _)| *x == s).map_or(0, |(_, m)| *m)
    }

    pub fn set_render_exception(&mut self, id: uuid::Uuid, mode: u8) {
        let s = id.to_string();
        self.render_exceptions.retain(|(x, _)| *x != s);
        if (1..=2).contains(&mode) {
            self.render_exceptions.push((s, mode));
        }
    }

    pub fn render_exceptions_map(&self) -> std::collections::HashMap<uuid::Uuid, u8> {
        self.render_exceptions
            .iter()
            .filter_map(|(id, m)| Some((uuid::Uuid::parse_str(id).ok()?, *m)))
            .collect()
    }

    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        let s = std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default();
        s.sanitized()
    }

    fn sanitized(mut self) -> Settings {
        if self.settings_version < 2 {
            // new defaults: 96 m draw distance, LOD factor 2
            self.draw_distance = 96.0;
            self.lod_factor = 2.0;
        }
        if self.settings_version < 3 && self.font == "Inter" {
            // Noto Sans became the default (consistent with the Noto emoji)
            self.font = "Noto Sans".into();
        }
        if self.force_midday {
            // former "always noon" switch
            self.time_of_day = 2;
            self.force_midday = false;
        }
        self.keybinds = std::mem::take(&mut self.keybinds).sanitized();
        self.camera = std::mem::take(&mut self.camera).sanitized();
        if self.settings_version < 5 {
            // new defaults: no vsync, no frame cap (the old "0 = screen rate,
            // 1 = unlimited" setting becomes a switch + value)
            self.vsync = false;
            self.fps_cap = self.fps_limit >= 2;
            if !self.fps_cap {
                self.fps_limit = 60;
            }
        }
        self.fps_limit = self.fps_limit.clamp(10, 500);
        self.background_fps_limit = self.background_fps_limit.clamp(1, 120);
        if self.settings_version < 6 {
            // new defaults: texture memory from the detected VRAM, 20 GB disk cache
            self.texture_budget_auto = true;
            if self.cache_size_mb == 4096 {
                self.cache_size_mb = 20480;
            }
        }
        if self.settings_version < 7 {
            // new default: parcel music waits for the radio toggle (Firestorm)
            self.audio.music_autoplay = false;
        }
        if self.settings_version < 8 && self.colors.chat_slurl == hex(0x6F7BA0) {
            // new default: SL links (places, avatars, groups) in the link teal
            self.colors.chat_slurl = ColorSettings::default().chat_slurl;
        }
        if self.settings_version < 9 {
            // new default: the background cap is on (a window left behind no
            // longer keeps the GPU and the desktop busy)
            self.background_fps_cap = true;
        }
        if self.settings_version < 4 {
            // walking: arrow keys only (no more WASD / ZQSD by default)
            self.keybinds.drop_letter_walk();
        }
        if !self.audio.mic_gain.is_finite() {
            self.audio.mic_gain = 1.0;
        }
        self.audio.mic_gain = self.audio.mic_gain.clamp(0.0, 2.0);
        self.time_of_day = self.time_of_day.min(4);
        self.env_manual_transition = if self.env_manual_transition.is_finite() {
            self.env_manual_transition.clamp(0.0, 60.0)
        } else {
            0.0
        };
        self.settings_version = SETTINGS_VERSION;
        self.max_particles = self.max_particles.min(8192);
        self.max_avatars = self.max_avatars.clamp(1, 100);
        self.complexity_mode = self.complexity_mode.min(2);
        self.loading_avatars = self.loading_avatars.min(2);
        self.max_attachment_area = if self.max_attachment_area.is_finite() {
            self.max_attachment_area.clamp(0.0, 100_000.0)
        } else {
            1000.0
        };
        self.render_exceptions
            .retain(|(id, m)| uuid::Uuid::parse_str(id).is_ok() && (1..=2).contains(m));
        self.ssao = self.ssao.min(2);
        self.water_reflections = self.water_reflections.min(4);
        self.mirrors = self.mirrors.min(4);
        self.anisotropy = match self.anisotropy {
            0 | 1 => 1,
            2 | 3 => 2,
            4..=7 => 4,
            8..=15 => 8,
            _ => 16,
        };
        self.cache_size_mb = self.cache_size_mb.clamp(512, 65536);
        if !self.chat_bar_width.is_finite() || self.chat_bar_width < 0.0 {
            self.chat_bar_width = 0.0;
        }
        self.draw_distance = self.draw_distance.clamp(32.0, 512.0);
        self.chat_commands.sanitize();
        self.exposure = self.exposure.clamp(0.25, 4.0);
        self.sharpen = if self.sharpen.is_finite() {
            self.sharpen.clamp(0.0, 1.0)
        } else {
            0.4
        };
        self.lod_factor = self.lod_factor.clamp(0.5, 4.0);
        self.ui_scale = self.ui_scale.clamp(0.75, 2.0);
        self.mouse_sensitivity = self.mouse_sensitivity.clamp(0.1, 5.0);
        self.texture_budget_mb = self.texture_budget_mb.clamp(256, 32768);
        self.font_scale = self.font_scale.clamp(0.8, 1.4);
        self.shadow_quality = self.shadow_quality.min(4);
        self.antialiasing = self.antialiasing.min(6);
        self.reflection_probes = self.reflection_probes.min(3);
        self.probe_count = self.probe_count.clamp(8, 64);
        self.tonemapper = self.tonemapper.min(1);
        self.chat_toast_seconds = self.chat_toast_seconds.clamp(0.0, 60.0);
        self.name_tag_distance = self.name_tag_distance.clamp(0.0, 128.0);
        if uuid::Uuid::parse_str(&self.machine_id).is_err() {
            self.machine_id = uuid::Uuid::new_v4().to_string();
        }
        self
    }

    /// Back to the defaults ("Tout réinitialiser"), keeping the account and
    /// grid used to log in, the start location and the window size.
    pub fn reset_keeping_account(&mut self) {
        let old = std::mem::take(self);
        *self = Settings {
            username: old.username,
            remember_username: old.remember_username,
            grid: old.grid,
            custom_login_uri: old.custom_login_uri,
            start_location: old.start_location,
            start_region: old.start_region,
            window_width: old.window_width,
            window_height: old.window_height,
            window_x: old.window_x,
            window_y: old.window_y,
            window_maximized: old.window_maximized,
            ..Settings::default()
        };
    }

    pub fn save(&self) {
        let mut copy = self.clone();
        if !copy.remember_username {
            copy.username.clear();
        }
        let dir = config_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("cannot create config dir: {e}");
            return;
        }
        match serde_json::to_vec_pretty(&copy) {
            Ok(b) => {
                if let Err(e) = std::fs::write(Self::path(), b) {
                    log::warn!("cannot save settings: {e}");
                }
            }
            Err(e) => log::warn!("cannot serialize settings: {e}"),
        }
    }

    pub fn login_uri(&self) -> String {
        match self.grid {
            GridChoice::SecondLife => aurora_net::login::SL_MAIN_GRID.into(),
            GridChoice::SecondLifeBeta => aurora_net::login::SL_BETA_GRID.into(),
            GridChoice::Custom => self.custom_login_uri.trim().to_owned(),
        }
    }

    /// Texture memory budget (MB): the manual value, or in automatic mode a
    /// share of the detected video memory (as Firestorm's unlimited VRAM
    /// mode: leave room for render targets, geometry and other programs).
    pub fn texture_budget(&self, vram_mb: Option<u64>, integrated: bool) -> u64 {
        if !self.texture_budget_auto {
            return self.texture_budget_mb as u64;
        }
        auto_texture_budget(vram_mb, integrated)
    }

    /// (resolution, distance) for the shadow quality step.
    pub fn shadow_params(&self) -> (u32, f32) {
        match self.shadow_quality {
            0 => (0, 0.0),
            1 => (1024, 48.0),
            2 => (2048, 96.0),
            3 => (2048, 192.0),
            _ => (4096, 256.0),
        }
    }

    pub fn render_settings(&self) -> aurora_render::RenderSettings {
        use aurora_render::{AntiAliasing, Tonemapper};
        aurora_render::RenderSettings {
            aa: match self.antialiasing {
                0 => AntiAliasing::None,
                1 => AntiAliasing::Fxaa,
                2 => AntiAliasing::Msaa(2),
                3 => AntiAliasing::Msaa(4),
                4 => AntiAliasing::Msaa(8),
                5 => AntiAliasing::Taa,
                _ => AntiAliasing::Smaa,
            },
            tonemapper: if self.tonemapper == 1 {
                Tonemapper::Aces
            } else {
                Tonemapper::KhronosNeutral
            },
            shadow_resolution: self.shadow_params().0,
            ssao: self.ssao,
            water_reflection_scale: Self::reflection_scale(self.water_reflections),
            mirror_scale: Self::reflection_scale(self.mirrors),
            probe_slots: self.probe_count.clamp(8, 64),
            ssr: self.ssr,
            anisotropy: self.anisotropy,
            occlusion: self.occlusion_culling,
        }
    }

    /// Fraction of the screen resolution for a reflection quality step.
    pub fn reflection_scale(step: u8) -> f32 {
        match step {
            0 => 0.0,
            1 => 0.25,
            2 => 0.5,
            3 => 0.75,
            _ => 1.0,
        }
    }

    /// Hashed identifiers sent at login (`mac`, `id0`).
    pub fn hashed_ids(&self) -> (String, String) {
        let mac = format!("{:x}", md5::compute(format!("mac:{}", self.machine_id)));
        let id0 = format!("{:x}", md5::compute(format!("id0:{}", self.machine_id)));
        (mac, id0)
    }
}

/// Mini-map (LLNetMap) and world map (LLFloaterWorldMap) options, with the
/// Firestorm defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MapSettings {
    /// MiniMapScale: pixels per region, 32..4096.
    pub mini_scale: f32,
    /// MiniMapRotate: camera direction at the top (else north).
    pub mini_rotate: bool,
    /// MiniMapObjects.
    pub mini_objects: bool,
    /// FSNetMapPhysical / FSNetMapScripted / FSNetMapTempOnRez.
    pub mini_physical: bool,
    pub mini_scripted: bool,
    pub mini_temp_on_rez: bool,
    /// MiniMapShowPropertyLines / MiniMapForSaleParcels.
    pub mini_property_lines: bool,
    pub mini_for_sale: bool,
    /// MiniMapCollisionParcels: parcels we may not enter, filled in red.
    pub mini_collision: bool,
    /// MiniMapAutoCenter.
    pub mini_auto_center: bool,
    /// FSNetMapDoubleClickAction: 0 nothing, 1 world map, 2 teleport.
    pub mini_double_click: u8,
    /// MapScale: world map pixels per region.
    pub world_scale: f32,
    /// MapShowPeople / MapShowInfohubs / MapShowTelehubs / MapShowLandForSale.
    pub world_people: bool,
    pub world_infohubs: bool,
    pub world_telehubs: bool,
    pub world_land_for_sale: bool,
    /// MapShowEvents / ShowMatureEvents / ShowAdultEvents.
    pub world_events: bool,
    pub world_mature_events: bool,
    pub world_adult_events: bool,
    /// MapShowGridCoords.
    pub world_grid_coords: bool,
    /// FSWorldMapDoubleclickTeleport.
    pub world_double_click_tp: bool,
    /// ShowBanLines: 0 hidden, 1 on collision, 2 on proximity.
    pub ban_lines: u8,
}

impl Default for MapSettings {
    fn default() -> Self {
        MapSettings {
            mini_scale: 128.0,
            // forced to north-up at the first login (llnetmap.cpp)
            mini_rotate: false,
            mini_objects: true,
            mini_physical: false,
            mini_scripted: false,
            mini_temp_on_rez: false,
            mini_property_lines: true,
            mini_for_sale: false,
            mini_collision: true,
            mini_auto_center: true,
            mini_double_click: 2,
            world_scale: 128.0,
            world_people: true,
            world_infohubs: true,
            world_telehubs: true,
            world_land_for_sale: true,
            world_events: true,
            world_mature_events: true,
            world_adult_events: false,
            world_grid_coords: false,
            world_double_click_tp: true,
            ban_lines: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::{Action, Input};
    use winit::keyboard::KeyCode;

    #[test]
    fn fps_preferences_remain_optional_and_keep_saved_limits() {
        assert!(!Settings::default().sanitized().fps_cap);
        for enabled in [false, true] {
            for limit in [30, 60, 144, 500] {
                let original = Settings {
                    fps_cap: enabled,
                    fps_limit: limit,
                    background_fps_limit: 120,
                    ..Settings::default()
                };
                let bytes = serde_json::to_vec(&original).expect("serialize synthetic settings");
                let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
                let saved = saved.sanitized();
                assert_eq!(saved.fps_cap, enabled);
                assert_eq!(saved.fps_limit, limit);
                assert_eq!(saved.background_fps_limit, 120);
            }
        }
    }

    #[test]
    fn hud_drag_key_defaults_for_old_files_and_keeps_the_saved_choice() {
        let old: Settings = serde_json::from_str("{}").expect("old synthetic settings");
        assert_eq!(old.sanitized().hud_drag_key, crate::keybinds::HoldKey::Alt);
        for key in [crate::keybinds::HoldKey::Ctrl, crate::keybinds::HoldKey::Key("KeyB".into())] {
            let settings = Settings {
                hud_drag_key: key.clone(),
                ..Default::default()
            };
            let bytes = serde_json::to_vec(&settings).expect("serialize synthetic settings");
            let loaded: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
            assert_eq!(loaded.sanitized().hud_drag_key, key);
        }
    }

    #[test]
    fn first_launch_movement_defaults_survive_saving_and_loading() {
        let fresh = Settings::default().sanitized();
        let bytes = serde_json::to_vec(&fresh).expect("serialize synthetic settings");
        let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
        assert_eq!(saved.sanitized().keybinds, fresh.keybinds);
        for action in [Action::Forward, Action::Back, Action::Left, Action::Right] {
            assert!(fresh.keybinds.get(action)[1].is_some());
        }
    }

    #[test]
    fn existing_settings_do_not_gain_or_replace_movement_bindings() {
        let mut existing = Settings::default();
        for action in [Action::Forward, Action::Back, Action::Left, Action::Right] {
            existing.keybinds.set(action, 1, None);
        }
        existing.keybinds.set(
            Action::Forward,
            1,
            Some(crate::keybinds::Binding {
                input: Input::key(KeyCode::KeyT),
                ctrl: false,
                shift: false,
                alt: false,
            }),
        );
        let bytes = serde_json::to_vec(&existing).expect("serialize synthetic settings");
        let saved: Settings = serde_json::from_slice(&bytes).expect("read synthetic settings");
        assert_eq!(saved.sanitized().keybinds, existing.keybinds);
    }

    #[test]
    fn parcel_music_waits_for_the_radio_toggle() {
        assert!(!Settings::default().sanitized().audio.music_autoplay);
        let mut old = Settings {
            settings_version: 6,
            ..Settings::default()
        };
        old.audio.music_autoplay = true;
        assert!(!old.sanitized().audio.music_autoplay);
        // switched back on after the migration: kept
        let mut on = Settings::default();
        on.audio.music_autoplay = true;
        assert!(on.sanitized().audio.music_autoplay);
    }

    #[test]
    fn background_cap_is_on_by_default() {
        assert!(Settings::default().sanitized().background_fps_cap);
        let old = Settings {
            settings_version: 8,
            background_fps_cap: false,
            ..Settings::default()
        };
        assert!(old.sanitized().background_fps_cap);
        // switched off after the migration: kept
        let off = Settings {
            background_fps_cap: false,
            ..Settings::default()
        };
        assert!(!off.sanitized().background_fps_cap);
    }
}
