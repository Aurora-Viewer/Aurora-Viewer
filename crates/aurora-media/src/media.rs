//! One media instance (LLPluginClassMedia): size negotiation with the
//! plugin, shared-memory pixels, dirty rectangles, status / navigation /
//! time events, input forwarding and transport controls.
//!
//! Port of indra/llplugin/llpluginclassmedia.cpp (Copyright (C) Linden
//! Research, Inc., originally LGPL 2.1).

use crate::message::PluginMessage;
use crate::process::{PluginPaths, PluginProcess};
use aurora_llsd::Llsd;
use std::collections::VecDeque;

pub const CLASS_MEDIA: &str = "media";
pub const CLASS_MEDIA_BROWSER: &str = "media_browser";
pub const CLASS_MEDIA_TIME: &str = "media_time";

/// LOW_PRIORITY_TEXTURE_SIZE_DEFAULT
const LOW_PRIORITY_TEXTURE_SIZE_DEFAULT: i32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Unloaded,
    Stopped,
    Hidden,
    Slideshow,
    Low,
    Normal,
    High,
}

impl Priority {
    fn as_str(self) -> &'static str {
        match self {
            Priority::Unloaded => "unloaded",
            Priority::Stopped => "stopped",
            Priority::Hidden => "hidden",
            Priority::Slideshow => "slideshow",
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
        }
    }

    /// Plugin sleep time for this priority (setPriority).
    fn sleep_time(self) -> f64 {
        match self {
            Priority::Low => 1.0 / 25.0,
            Priority::Normal => 1.0 / 50.0,
            Priority::High => 1.0 / 100.0,
            _ => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaStatus {
    #[default]
    None,
    Loading,
    Loaded,
    Error,
    Playing,
    Paused,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Down,
    Up,
    Move,
    DoubleClick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Down,
    Up,
    Repeat,
}

/// Modifier keys (translateModifiers).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    fn translate(self) -> String {
        let mut s = String::new();
        if self.control {
            s.push_str("control|");
        }
        if self.alt {
            s.push_str("alt|");
        }
        if self.shift {
            s.push_str("shift|");
        }
        s
    }
}

/// Events for the owner (LLPluginClassMediaOwner::EMediaEvent).
#[derive(Debug, Clone, PartialEq)]
pub enum MediaEvent {
    ContentUpdated,
    SizeChanged,
    StatusChanged(MediaStatus),
    NavigateBegin(String),
    NavigateComplete { uri: String, code: i32 },
    LocationChanged(String),
    NameChanged(String),
    ClickLinkHref { url: String, target: String },
    ClickLinkNoFollow { url: String, nav_type: String },
    CloseRequest,
    CursorChanged(String),
    StatusText(String),
    Progress(i32),
    TimeDurationUpdated,
    NavigateErrorPage(i32),
    PluginFailed(String),
    DebugMessage(String),
}

/// Dirty rectangle, in plugin coordinates (OpenGL style when
/// `coords_opengl`: bottom > top is swapped so that top >= bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    fn union(self, o: Rect) -> Rect {
        Rect {
            left: self.left.min(o.left),
            top: self.top.max(o.top),
            right: self.right.max(o.right),
            bottom: self.bottom.min(o.bottom),
        }
    }
}

/// Pixels of the current frame (BGRA, 4 bytes per pixel).
pub struct Frame<'a> {
    pub media_width: u32,
    pub media_height: u32,
    /// Row length in pixels of the shared buffer.
    pub texture_width: u32,
    pub texture_height: u32,
    /// Rows run bottom-up (OpenGL coordinates).
    pub bottom_up: bool,
    pub data: &'a [u8],
}

pub fn next_power_of_2(v: i32) -> i32 {
    if v <= 1 {
        return 1;
    }
    (v as u32).next_power_of_two() as i32
}

/// Browser settings sent before `init` (newSourceFromMediaType).
#[derive(Debug, Clone)]
pub struct BrowserSettings {
    pub cache_path: String,
    pub username: String,
    pub cef_log_file: String,
    pub language: String,
    pub user_agent: String,
    pub cookies_enabled: bool,
    pub javascript_enabled: bool,
    pub zoom_factor: f64,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        BrowserSettings {
            cache_path: String::new(),
            username: "default".into(),
            cef_log_file: String::new(),
            language: "fr".into(),
            user_agent: String::new(),
            cookies_enabled: true,
            javascript_enabled: true,
            zoom_factor: 1.0,
        }
    }
}

pub struct MediaPlugin {
    process: PluginProcess,
    queue: VecDeque<PluginMessage>,
    texture_params_received: bool,
    depth: i32,
    format: u32,
    coords_opengl: bool,
    default_width: i32,
    default_height: i32,
    allow_downsample: bool,
    padding: i32,
    natural_width: i32,
    natural_height: i32,
    set_width: i32,
    set_height: i32,
    requested_media_width: i32,
    requested_media_height: i32,
    requested_texture_width: i32,
    requested_texture_height: i32,
    full_width: i32,
    full_height: i32,
    texture_width: i32,
    texture_height: i32,
    media_width: i32,
    media_height: i32,
    shm_name: Option<String>,
    shm_size: usize,
    dirty: Option<Rect>,
    auto_scale: bool,
    priority: Priority,
    low_priority_size_limit: i32,
    requested_volume: f32,
    status: MediaStatus,
    events: Vec<MediaEvent>,
    failed: bool,
    pub plugin_name: String,
    // media_browser
    pub location: String,
    pub media_name: String,
    pub history_back: bool,
    pub history_forward: bool,
    pub progress: i32,
    pub status_text: String,
    pub cursor: String,
    pub hover_text: String,
    // media_time
    pub current_time: f64,
    pub duration: f64,
    pub current_rate: f64,
    pub loaded_duration: f64,
}

impl MediaPlugin {
    /// Launch a plugin (`media_plugin_cef` / `media_plugin_libvlc`) sized
    /// `width` × `height` (0 = plugin default).
    pub fn launch(
        paths: &PluginPaths,
        plugin: &str,
        width: i32,
        height: i32,
        browser: &BrowserSettings,
        target: &str,
    ) -> std::io::Result<MediaPlugin> {
        let file = paths.plugin_file(plugin);
        if !file.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found", file.display()),
            ));
        }
        let process = PluginProcess::launch(&paths.launcher, &paths.plugin_dir, &file)?;
        let mut m = MediaPlugin {
            process,
            queue: VecDeque::new(),
            texture_params_received: false,
            depth: 0,
            format: 0,
            coords_opengl: false,
            default_width: 0,
            default_height: 0,
            allow_downsample: false,
            padding: 0,
            natural_width: 0,
            natural_height: 0,
            set_width: -1,
            set_height: -1,
            requested_media_width: 0,
            requested_media_height: 0,
            requested_texture_width: 0,
            requested_texture_height: 0,
            full_width: 0,
            full_height: 0,
            texture_width: 0,
            texture_height: 0,
            media_width: 0,
            media_height: 0,
            shm_name: None,
            shm_size: 0,
            dirty: None,
            auto_scale: false,
            priority: Priority::Normal,
            low_priority_size_limit: LOW_PRIORITY_TEXTURE_SIZE_DEFAULT,
            requested_volume: 0.0,
            status: MediaStatus::None,
            events: Vec::new(),
            failed: false,
            plugin_name: plugin.to_string(),
            location: String::new(),
            media_name: String::new(),
            history_back: false,
            history_forward: false,
            progress: 0,
            status_text: String::new(),
            cursor: String::new(),
            hover_text: String::new(),
            current_time: 0.0,
            duration: 0.0,
            current_rate: 0.0,
            loaded_duration: 0.0,
        };
        m.set_size(width, height);
        // newSourceFromMediaType: settings first, the media init last
        m.send(
            PluginMessage::new(CLASS_MEDIA, "set_user_data_path")
                .with("cache_path", browser.cache_path.as_str())
                .with("username", browser.username.as_str())
                .with("cef_log_file", browser.cef_log_file.as_str())
                .with("cef_verbose_log", false),
        );
        m.send(PluginMessage::new(CLASS_MEDIA, "set_language_code").with("language", browser.language.as_str()));
        m.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "cookies_enabled").with("enable", browser.cookies_enabled));
        m.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "javascript_enabled").with("enable", browser.javascript_enabled));
        m.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "web_security_disabled").with("disabled", false));
        m.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "file_access_from_file_urls").with("enabled", false));
        m.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "plugins_enabled").with("enable", true));
        m.send(PluginMessage::new(CLASS_MEDIA, "enable_media_plugin_debugging").with("enable", false));
        if !browser.user_agent.is_empty() {
            m.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "set_user_agent").with("user_agent", browser.user_agent.as_str()));
        }
        m.send(
            PluginMessage::new(CLASS_MEDIA_BROWSER, "proxy_setup")
                .with("enable", false)
                .with("host", "")
                .with("port", 0),
        );
        m.send(
            PluginMessage::new(CLASS_MEDIA, "init")
                .with("target", target)
                .with("factor", browser.zoom_factor),
        );
        Ok(m)
    }

    fn send(&mut self, m: PluginMessage) {
        self.queue.push_back(m);
    }

    pub fn status(&self) -> MediaStatus {
        self.status
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn is_running(&self) -> bool {
        self.process.is_running()
    }

    pub fn cpu_usage(&self) -> f64 {
        self.process.cpu_usage
    }

    pub fn take_events(&mut self) -> Vec<MediaEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    pub fn full_size(&self) -> (i32, i32) {
        (self.full_width, self.full_height)
    }

    pub fn media_size(&self) -> (i32, i32) {
        (self.media_width, self.media_height)
    }

    /// Texture size as LL allocates it (power of two).
    pub fn texture_size(&self) -> (i32, i32) {
        (next_power_of_2(self.texture_width), next_power_of_2(self.texture_height))
    }

    pub fn idle(&mut self) {
        self.process.idle();
        if let Some(err) = self.process.error() {
            if !self.failed {
                self.failed = true;
                self.events.push(MediaEvent::PluginFailed(err.to_string()));
            }
            return;
        }
        for m in self.process.take_messages() {
            self.receive(m);
        }
        if self.media_width == -1 || !self.texture_params_received || !self.process.is_running() {
            // can't process a size change at this time
        } else if self.requested_media_width != self.media_width || self.requested_media_height != self.media_height {
            self.start_size_change();
        }
        if self.process.is_running() {
            while let Some(m) = self.queue.pop_front() {
                self.process.send(&m);
            }
        }
    }

    fn start_size_change(&mut self) {
        self.requested_texture_height = self.requested_media_height;
        if self.padding < 0 {
            // negative values: the plugin wants a power of 2
            self.requested_texture_width = next_power_of_2(self.requested_media_width);
        } else {
            self.requested_texture_width = self.requested_media_width;
            if self.padding > 1 {
                let mut rowbytes = self.requested_texture_width * self.depth;
                let pad = rowbytes % self.padding;
                if pad != 0 {
                    rowbytes += self.padding - pad;
                }
                if self.depth > 0 && rowbytes % self.depth == 0 {
                    self.requested_texture_width = rowbytes / self.depth;
                }
            }
        }
        let depth = self.depth.max(1) as usize;
        // plus an extra line of padding, just in case
        let newsize = (self.requested_texture_width.max(1) as usize) * (self.requested_texture_height.max(1) as usize + 1) * depth;
        if newsize != self.shm_size {
            if let Some(old) = self.shm_name.take() {
                self.process.remove_shared_memory(&old);
            }
            self.shm_size = newsize;
            self.shm_name = self.process.add_shared_memory(newsize);
        }
        self.texture_width = -1;
        self.texture_height = -1;
        self.media_width = -1;
        self.media_height = -1;
        self.dirty = None;
        // sent directly: jumps ahead of the queue
        let m = PluginMessage::new(CLASS_MEDIA, "size_change")
            .with("name", self.shm_name.clone().unwrap_or_default())
            .with("width", self.requested_media_width)
            .with("height", self.requested_media_height)
            .with("texture_width", self.requested_texture_width)
            .with("texture_height", self.requested_texture_height)
            .with("background_r", 1.0)
            .with("background_g", 1.0)
            .with("background_b", 1.0)
            .with("background_a", 1.0);
        self.process.send(&m);
    }

    /// setSize: 0 = natural / default size.
    pub fn set_size(&mut self, width: i32, height: i32) {
        if width > 0 && height > 0 {
            self.set_width = width;
            self.set_height = height;
        } else {
            self.set_width = -1;
            self.set_height = -1;
        }
        self.set_size_internal();
    }

    fn set_size_internal(&mut self) {
        if self.set_width > 0 && self.set_height > 0 {
            self.requested_media_width = self.set_width;
            self.requested_media_height = self.set_height;
        } else if self.natural_width > 0 && self.natural_height > 0 {
            self.requested_media_width = self.natural_width;
            self.requested_media_height = self.natural_height;
        } else {
            self.requested_media_width = self.default_width;
            self.requested_media_height = self.default_height;
        }
        self.full_width = self.requested_media_width;
        self.full_height = self.requested_media_height;
        if self.allow_downsample && matches!(self.priority, Priority::Slideshow | Priority::Low) {
            while self.requested_media_width > self.low_priority_size_limit || self.requested_media_height > self.low_priority_size_limit {
                self.requested_media_width /= 2;
                self.requested_media_height /= 2;
            }
        }
        if self.auto_scale {
            self.requested_media_width = next_power_of_2(self.requested_media_width);
            self.requested_media_height = next_power_of_2(self.requested_media_height);
        }
        self.requested_media_width = self.requested_media_width.min(2048);
        self.requested_media_height = self.requested_media_height.min(2048);
    }

    pub fn set_auto_scale(&mut self, auto_scale: bool) {
        if auto_scale != self.auto_scale {
            self.auto_scale = auto_scale;
            self.set_size_internal();
        }
    }

    pub fn set_low_priority_size_limit(&mut self, size: i32) {
        let size = next_power_of_2(size.max(1));
        if size != self.low_priority_size_limit {
            self.low_priority_size_limit = size;
            self.set_size_internal();
        }
    }

    pub fn set_priority(&mut self, p: Priority) {
        if p == self.priority {
            return;
        }
        self.priority = p;
        self.send(PluginMessage::new(CLASS_MEDIA, "set_priority").with("priority", p.as_str()));
        self.process.set_sleep_time(p.sleep_time(), false);
        self.set_size_internal();
    }

    /// textureValid
    pub fn texture_valid(&self) -> bool {
        self.texture_params_received
            && self.texture_width > 0
            && self.texture_height > 0
            && self.media_width > 0
            && self.media_height > 0
            && self.requested_media_width == self.media_width
            && self.requested_media_height == self.media_height
            && self.shm_name.as_deref().and_then(|n| self.process.shared_memory(n)).is_some()
    }

    /// Dirty rectangle since the last `reset_dirty`.
    pub fn dirty(&self) -> Option<Rect> {
        self.dirty
    }

    pub fn reset_dirty(&mut self) {
        self.dirty = None;
    }

    pub fn frame(&self) -> Option<Frame<'_>> {
        if !self.texture_valid() {
            return None;
        }
        let shm = self.process.shared_memory(self.shm_name.as_deref()?)?;
        if self.depth != 4 {
            return None;
        }
        Some(Frame {
            media_width: self.media_width as u32,
            media_height: self.media_height as u32,
            texture_width: self.texture_width as u32,
            texture_height: self.texture_height as u32,
            bottom_up: self.coords_opengl,
            data: shm.bytes(),
        })
    }

    /// Plugin pixel format is BGRA (GL_BGRA / GL_BGRA_EXT).
    pub fn is_bgra(&self) -> bool {
        self.format == 0x80E1
    }

    // ---------------------------------------------------------------- input

    /// Mouse event; `x`, `y` in media pixels, y from the top.
    pub fn mouse_event(&mut self, ev: MouseEvent, button: i32, x: i32, y: i32, mods: Modifiers) {
        let name = match ev {
            MouseEvent::Down => "down",
            MouseEvent::Up => "up",
            MouseEvent::Move => "move",
            MouseEvent::DoubleClick => "double_click",
        };
        // the incoming coordinates are top-down already; LL flips OpenGL
        // style ones for plugins not asking for OpenGL coordinates
        let m = PluginMessage::new(CLASS_MEDIA, "mouse_event")
            .with("event", name)
            .with("button", button)
            .with("x", x)
            .with("y", y)
            .with("modifiers", mods.translate());
        self.send(m);
    }

    pub fn scroll_event(&mut self, x: i32, y: i32, clicks_x: i32, clicks_y: i32, mods: Modifiers) {
        self.send(
            PluginMessage::new(CLASS_MEDIA, "scroll_event")
                .with("x", x)
                .with("y", y)
                .with("clicks_x", clicks_x)
                .with("clicks_y", clicks_y)
                .with("modifiers", mods.translate()),
        );
    }

    /// Windows native key data: the message (WM_KEYDOWN / WM_KEYUP / WM_CHAR),
    /// wParam and lParam, each as a 4-byte big-endian binary (ll_sd_from_U32).
    pub fn native_key_data(msg: u32, wparam: u32, lparam: u32) -> Llsd {
        let mut m = Llsd::new_map();
        m.insert("msg", Llsd::Binary(msg.to_be_bytes().to_vec()));
        m.insert("w_param", Llsd::Binary(wparam.to_be_bytes().to_vec()));
        m.insert("l_param", Llsd::Binary(lparam.to_be_bytes().to_vec()));
        m
    }

    pub fn key_event(&mut self, ev: KeyEvent, key: i32, mods: Modifiers, native: Llsd) {
        let name = match ev {
            KeyEvent::Down => "down",
            KeyEvent::Up => "up",
            KeyEvent::Repeat => "repeat",
        };
        self.send(
            PluginMessage::new(CLASS_MEDIA, "key_event")
                .with("event", name)
                .with("key", key)
                .with("modifiers", mods.translate())
                .with("native_key_data", native),
        );
    }

    pub fn text_input(&mut self, text: &str, mods: Modifiers, native: Llsd) {
        self.send(
            PluginMessage::new(CLASS_MEDIA, "text_event")
                .with("text", text)
                .with("modifiers", mods.translate())
                .with("native_key_data", native),
        );
    }

    pub fn focus(&mut self, focused: bool) {
        self.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "focus").with("focused", focused));
    }

    // ------------------------------------------------------------ navigation

    pub fn load_uri(&mut self, uri: &str) {
        self.send(PluginMessage::new(CLASS_MEDIA, "load_uri").with("uri", uri));
    }

    pub fn browse_stop(&mut self) {
        self.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "browse_stop"));
    }

    pub fn browse_reload(&mut self, ignore_cache: bool) {
        self.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "browse_reload").with("ignore_cache", ignore_cache));
    }

    pub fn browse_forward(&mut self) {
        self.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "browse_forward"));
    }

    pub fn browse_back(&mut self) {
        self.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "browse_back"));
    }

    pub fn set_page_zoom(&mut self, factor: f64) {
        self.send(PluginMessage::new(CLASS_MEDIA_BROWSER, "set_page_zoom_factor").with("factor", factor));
    }

    // ------------------------------------------------------------ media_time

    pub fn stop(&mut self) {
        self.send(PluginMessage::new(CLASS_MEDIA_TIME, "stop"));
    }

    pub fn start(&mut self, rate: f64) {
        self.send(PluginMessage::new(CLASS_MEDIA_TIME, "start").with("rate", rate));
    }

    pub fn pause(&mut self) {
        self.send(PluginMessage::new(CLASS_MEDIA_TIME, "pause"));
    }

    pub fn seek(&mut self, time: f64) {
        self.send(PluginMessage::new(CLASS_MEDIA_TIME, "seek").with("time", time));
    }

    pub fn set_loop(&mut self, looping: bool) {
        self.send(PluginMessage::new(CLASS_MEDIA_TIME, "set_loop").with("loop", looping));
    }

    /// Volume 0..1 (sent only when it changes).
    pub fn set_volume(&mut self, volume: f32) {
        if (volume - self.requested_volume).abs() > 0.001 {
            self.requested_volume = volume;
            self.send(PluginMessage::new(CLASS_MEDIA_TIME, "set_volume").with("volume", volume as f64));
        }
    }

    pub fn shutdown(&mut self) {
        self.process.shutdown();
    }

    // -------------------------------------------------------------- receive

    fn receive(&mut self, m: PluginMessage) {
        match m.class.as_str() {
            CLASS_MEDIA => self.receive_media(m),
            CLASS_MEDIA_BROWSER => self.receive_browser(m),
            _ => {}
        }
    }

    fn receive_media(&mut self, m: PluginMessage) {
        match m.name.as_str() {
            "texture_params" => {
                self.depth = m.s32("depth");
                self.format = m.u32_hex("format");
                self.coords_opengl = m.boolean("coords_opengl");
                self.default_width = m.s32("default_width");
                self.default_height = m.s32("default_height");
                self.allow_downsample = m.boolean("allow_downsample");
                self.padding = m.s32("padding");
                self.set_size_internal();
                self.texture_params_received = true;
            }
            "updated" => {
                if m.has("left") {
                    let mut r = Rect {
                        left: m.s32("left"),
                        top: m.s32("top"),
                        right: m.s32("right"),
                        bottom: m.s32("bottom"),
                    };
                    // top and bottom may be switched (vertical flip)
                    if r.top < r.bottom {
                        std::mem::swap(&mut r.top, &mut r.bottom);
                    }
                    self.dirty = Some(match self.dirty {
                        Some(d) => d.union(r),
                        None => r,
                    });
                    self.events.push(MediaEvent::ContentUpdated);
                }
                let mut time_updated = false;
                if m.has("current_time") {
                    self.current_time = m.real("current_time");
                    time_updated = true;
                }
                if m.has("duration") {
                    self.duration = m.real("duration");
                    time_updated = true;
                }
                if m.has("current_rate") {
                    self.current_rate = m.real("current_rate");
                }
                if m.has("loaded_duration") {
                    self.loaded_duration = m.real("loaded_duration");
                    time_updated = true;
                } else {
                    self.loaded_duration = self.duration;
                }
                if time_updated {
                    self.events.push(MediaEvent::TimeDurationUpdated);
                }
            }
            "media_status" => {
                self.status = match m.string("status").as_str() {
                    "loading" => MediaStatus::Loading,
                    "loaded" => MediaStatus::Loaded,
                    "error" => MediaStatus::Error,
                    "playing" => MediaStatus::Playing,
                    "paused" => MediaStatus::Paused,
                    "done" => MediaStatus::Done,
                    _ => MediaStatus::None,
                };
                self.events.push(MediaEvent::StatusChanged(self.status));
            }
            "size_change_request" => {
                self.natural_width = m.s32("width");
                self.natural_height = m.s32("height");
                self.set_size_internal();
            }
            "size_change_response" => {
                self.texture_width = m.s32("texture_width");
                self.texture_height = m.s32("texture_height");
                self.media_width = m.s32("width");
                self.media_height = m.s32("height");
                self.dirty = None;
                self.events.push(MediaEvent::SizeChanged);
            }
            "cursor_changed" => {
                self.cursor = m.string("name");
                self.events.push(MediaEvent::CursorChanged(self.cursor.clone()));
            }
            "name_text" => {
                self.history_back = m.boolean("history_back_available");
                self.history_forward = m.boolean("history_forward_available");
                self.media_name = m.string("name");
                self.events.push(MediaEvent::NameChanged(self.media_name.clone()));
            }
            "tooltip_text" => self.hover_text = m.string("tooltip"),
            "pick_file" => {
                // blocking request: answer with nothing picked
                let r = PluginMessage::new(CLASS_MEDIA, "pick_file_response")
                    .with("file_list", Llsd::new_array())
                    .with("blocking_response", true);
                self.process.send(&r);
            }
            "auth_request" => {
                let r = PluginMessage::new(CLASS_MEDIA, "auth_response")
                    .with("ok", false)
                    .with("username", "")
                    .with("password", "")
                    .with("blocking_response", true);
                self.process.send(&r);
            }
            "debug_message" => self.events.push(MediaEvent::DebugMessage(m.string("message_text"))),
            _ => {}
        }
    }

    fn receive_browser(&mut self, m: PluginMessage) {
        match m.name.as_str() {
            "navigate_begin" => {
                self.events.push(MediaEvent::NavigateBegin(m.string("uri")));
            }
            "navigate_complete" => {
                let uri = m.string("uri");
                self.history_back = m.boolean("history_back_available");
                self.history_forward = m.boolean("history_forward_available");
                if !uri.is_empty() {
                    self.location = uri.clone();
                }
                self.events.push(MediaEvent::NavigateComplete {
                    uri,
                    code: m.s32("result_code"),
                });
            }
            "progress" => {
                self.progress = m.s32("percent");
                self.events.push(MediaEvent::Progress(self.progress));
            }
            "status_text" => {
                self.status_text = m.string("status");
                self.events.push(MediaEvent::StatusText(self.status_text.clone()));
            }
            "location_changed" => {
                self.location = m.string("uri");
                self.events.push(MediaEvent::LocationChanged(self.location.clone()));
            }
            "click_href" => self.events.push(MediaEvent::ClickLinkHref {
                url: m.string("uri"),
                target: m.string("target"),
            }),
            "click_nofollow" => self.events.push(MediaEvent::ClickLinkNoFollow {
                url: m.string("uri"),
                nav_type: m.string("nav_type"),
            }),
            "navigate_error_page" => self.events.push(MediaEvent::NavigateErrorPage(m.s32("status_code"))),
            "close_request" => self.events.push(MediaEvent::CloseRequest),
            "link_hovered" => self.hover_text = m.string("title"),
            _ => {}
        }
    }
}

impl Drop for MediaPlugin {
    fn drop(&mut self) {
        self.process.shutdown();
    }
}

/// Plugin for a MIME type (LLMIMETypes::implType over mime_types.xml,
/// Windows variant): unknown types go to the browser.
pub fn plugin_for_mime(mime: &str) -> &'static str {
    let mime = mime.trim().to_ascii_lowercase();
    const CEF_EXCEPTIONS: [&str; 4] = ["application/ogg", "audio/mid", "video/x-ms-wmv", "application/smil"];
    if CEF_EXCEPTIONS.contains(&mime.as_str()) {
        return "media_plugin_cef";
    }
    if mime.starts_with("video/") || mime.starts_with("audio/") || mime == "application/octet-stream" || mime == "rtsp" || mime == "libvlc"
    {
        return "media_plugin_libvlc";
    }
    "media_plugin_cef"
}

/// Media type from the URL alone (navigateInternal): data / file / about
/// pages are HTML, other non-HTTP schemes are their own key (rtsp → VLC).
/// None: an HTTP(S) URL whose type must be discovered (HEAD request).
pub fn mime_from_scheme(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let scheme = match lower.find(':') {
        Some(i) if i > 1 && lower[..i].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-') => &lower[..i],
        _ => return None,
    };
    match scheme {
        "http" | "https" | "" => None,
        "data" | "file" | "about" => Some("text/html".into()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_routing() {
        assert_eq!(plugin_for_mime("text/html"), "media_plugin_cef");
        assert_eq!(plugin_for_mime("video/mp4"), "media_plugin_libvlc");
        assert_eq!(plugin_for_mime("audio/mpeg"), "media_plugin_libvlc");
        assert_eq!(plugin_for_mime("video/x-ms-wmv"), "media_plugin_cef");
        assert_eq!(plugin_for_mime("rtsp"), "media_plugin_libvlc");
        assert_eq!(mime_from_scheme("rtsp://x/y").as_deref(), Some("rtsp"));
        assert_eq!(mime_from_scheme("https://x/y"), None);
        assert_eq!(mime_from_scheme("www.example.com"), None);
        assert_eq!(mime_from_scheme("data:text/html,hi").as_deref(), Some("text/html"));
    }
}
