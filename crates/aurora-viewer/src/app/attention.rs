//! What `world/attention.rs` records, decided and done: the taskbar flash
//! (LLWindowWin32::flashIcon, indra/llwindow/llwindowwin32.cpp) and the
//! system notification (GrowlManager::performNotification / shouldNotify,
//! indra/newview/growlmanager.cpp, originally LGPL 2.1), and the click on a
//! notification, which brings the window back to what it told.

use super::App;
use crate::keywords::KeywordSettings;
use crate::settings::Settings;
use crate::world::attention::{Cause, Notice, Open};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// GROWL_THROTTLE_TIME: a second notification with the same title within
/// this time is dropped.
const THROTTLE: Duration = Duration::from_secs(1);
/// GROWL_THROTTLE_CLEANUP_PERIOD.
const THROTTLE_CLEANUP: Duration = Duration::from_secs(300);
/// GROWL_MAX_BODY_LENGTH (characters here, bytes there).
const MAX_BODY: usize = 255;
/// The notifications whose click is still remembered.
const KEEP_OPENS: usize = 64;

#[derive(Default)]
pub struct State {
    /// Started with the first notification (the setting off: never).
    notifier: Option<crate::system_notify::Notifier>,
    throttle: Throttle,
    next_id: u64,
    /// What a click on each notification shown opens.
    opens: VecDeque<(u64, Open)>,
    /// A flash asked during the frame, done once the window is back.
    flash: bool,
    /// The window is not the foreground window, or is minimized, as
    /// Windows says between frames (LLWindowWin32::flashIcon asks
    /// GetForegroundWindow): the focus events can miss a minimize.
    behind: bool,
}

/// The titles told lately (GrowlManager::mTitleTimers).
#[derive(Default)]
struct Throttle {
    seen: HashMap<String, Instant>,
    cleaned: Option<Instant>,
}

impl Throttle {
    /// The title may be told now. A repeat within the second is dropped and
    /// starts the second again, as performNotification does.
    fn allow(&mut self, title: &str, now: Instant) -> bool {
        if self.cleaned.is_none_or(|t| now.duration_since(t) >= THROTTLE_CLEANUP) {
            self.seen.clear();
            self.cleaned = Some(now);
        }
        let repeat = self.seen.get(title).is_some_and(|t| now.duration_since(*t) < THROTTLE);
        self.seen.insert(title.to_owned(), now);
        !repeat
    }
}

/// Whether a cause flashes the taskbar, by the settings
/// (processNewMessage: FSFlashOnMessage, FSFlashOnObjectIM for the objects'
/// messages; LLScriptFloaterManager: FSFlashOnScriptDialog).
pub fn flashes(cause: Cause, s: &Settings) -> bool {
    match cause {
        Cause::Message | Cause::Mention => s.flash_on_message,
        Cause::ObjectIm => s.flash_on_message && s.flash_on_object_im,
        Cause::ScriptDialog => s.flash_on_script_dialog,
        // FSKeywords::notify plays its sound and tells Growl, no flash
        Cause::Keyword | Cause::Notice => false,
    }
}

/// The keyword sound to play (PlayModeUISndFSKeywordSound,
/// UISndFSKeywordSound), under the "Interface sounds" switch.
pub fn keyword_sound(s: &Settings, k: &KeywordSettings) -> Option<uuid::Uuid> {
    (s.audio.ui_sounds && k.play_sound && !k.sound.is_nil()).then_some(k.sound)
}

/// Whether the system is told now (GrowlManager::shouldNotify): turned on,
/// never in do not disturb, and only while the window is not in front
/// unless FSGrowlWhenActive. `forced`: turned on by AURORA_DEMO_ATTENTION.
pub fn notifies(s: &Settings, forced: bool, in_front: bool, dnd: bool) -> bool {
    (s.system_notifications || forced) && !dnd && (s.system_notifications_when_active || !in_front)
}

/// What a notification opens, without its session (for the log).
fn kind_of(open: Open) -> &'static str {
    match open {
        Open::Conversation(_) => "conversation",
        Open::NearbyChat => "nearby chat",
        Open::Notifications => "notifications",
        Open::Window => "window",
    }
}

/// The body as the system shows it: one line of plain text, cut at
/// GROWL_MAX_BODY_LENGTH.
fn body_text(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX_BODY) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

impl App {
    /// Each frame in world: what the world asked for since the last frame.
    pub(super) fn update_attention(&mut self) {
        let mut keyword_sound = None;
        for a in std::mem::take(&mut self.world.attention) {
            if self.attention.behind && flashes(a.cause, &self.settings) {
                self.flash_taskbar();
            }
            if a.cause == Cause::Keyword {
                // once a frame, however many lines arrived together
                keyword_sound = self::keyword_sound(&self.settings, &self.keyword_settings());
            }
            if let Some(notice) = a.notice {
                self.tell_system(notice);
            }
        }
        if let Some(id) = keyword_sound {
            log::debug!(target: "ui_sound", "UISndFSKeywordSound ({id})");
            self.scene.sounds.play_ui(self.audio_engine.as_ref(), id);
        }
    }

    /// The keyword alerts in force: AURORA_DEMO_KEYWORDS's for the run
    /// (never saved), else the preferences'.
    pub(super) fn keyword_settings(&self) -> KeywordSettings {
        match crate::demo::keywords::settings() {
            Some(k) if self.demo => k,
            _ => self.settings.keywords.clone(),
        }
    }

    /// One system notification, when the settings, the focus, do not
    /// disturb and the throttle let it through.
    pub(super) fn tell_system(&mut self, notice: Notice) {
        let forced = self.demo && crate::demo::attention::mode().is_some();
        if !notifies(&self.settings, forced, !self.attention.behind, self.world.status.dnd) {
            return;
        }
        let world = &self.world;
        let name = |id: &uuid::Uuid| world.social.avatar_names.complete(id);
        // a label still waiting for its name keeps the address: the
        // notification is not drawn again once it comes (#947)
        let app = |url: &str, link: &crate::slapp::AppLink| match crate::ui::chat::app_label(world, url, link) {
            label if label == "…" => crate::slapp::unescape(url),
            label => label,
        };
        let title = body_text(&crate::ui::chat::plain_text(&notice.title, name, app));
        if title.is_empty() {
            return;
        }
        if !self.attention.throttle.allow(&title, Instant::now()) {
            log::info!("system notification dropped by the throttle");
            return;
        }
        let body = body_text(&crate::ui::chat::plain_text(&notice.body, name, app));
        log::info!("system notification shown ({})", kind_of(notice.open));
        let st = &mut self.attention;
        st.next_id += 1;
        let id = st.next_id;
        st.opens.push_back((id, notice.open));
        if st.opens.len() > KEEP_OPENS {
            st.opens.pop_front();
        }
        st.notifier
            .get_or_insert_with(crate::system_notify::Notifier::start)
            .show(crate::system_notify::Toast { id, title, body });
    }

    /// Between frames, the window at hand (the frame holds it): whether it
    /// is in front, the flash asked for, and the clicks on the notifications.
    pub(super) fn attention_window(&mut self) {
        if let Some(g) = &self.gfx {
            self.attention.behind = !window_in_front(&g.window);
        }
        if std::mem::take(&mut self.attention.flash)
            && let Some(g) = &self.gfx
        {
            #[cfg(windows)]
            {
                log::info!("taskbar flash");
                flash_window(&g.window);
            }
            #[cfg(not(windows))]
            if flashes_here(g.window.is_minimized()) {
                log::info!("taskbar flash");
                g.window
                    .request_user_attention(Some(winit::window::UserAttentionType::Informational));
            } else {
                log::debug!("taskbar flash skipped: the window is not minimized");
            }
        }
        self.attention_clicks();
    }

    /// A notification clicked: the window comes back (restored, in front)
    /// on its conversation, the nearby chat or the bell's list.
    fn attention_clicks(&mut self) {
        let Some(notifier) = &self.attention.notifier else {
            return;
        };
        let mut clicked = Vec::new();
        while let Some(id) = notifier.clicked() {
            clicked.push(id);
        }
        for id in clicked {
            let open = self.attention.opens.iter().find(|(i, _)| *i == id).map(|(_, o)| *o);
            if let Some(g) = &self.gfx {
                g.window.set_minimized(false);
                g.window.focus_window();
            }
            if !matches!(self.screen, super::Screen::World) {
                continue;
            }
            match open {
                Some(Open::Conversation(session)) => {
                    if !self.world.rlva_can_start_im(session) {
                        continue;
                    }
                    self.world.social.session_mut(session);
                    self.world.social.focus_im = Some(session);
                    crate::ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
                }
                Some(Open::NearbyChat) => {
                    self.world.social.focus_im = Some(uuid::Uuid::nil());
                    crate::ui::notifications::open_conversations(&self.egui_ctx, &mut self.panels.chat);
                }
                Some(Open::Notifications) => self.notif_ui.open = true,
                Some(Open::Window) | None => {}
            }
        }
    }

    /// LLWindow::flashIcon(5.f): the taskbar button flashes ten times in
    /// five seconds, then stays lit until the window is activated
    /// (LLWindowWin32::flashIcon: FLASHW_TRAY, ICON_FLASH_TIME 0.5 s).
    pub(super) fn flash_taskbar(&mut self) {
        self.attention.flash = true;
    }

    /// The window is not in front (another window has the foreground, or
    /// it is minimized).
    pub(super) fn window_behind(&self) -> bool {
        self.attention.behind
    }
}

/// The window is the foreground window and not minimized.
#[cfg(windows)]
fn window_in_front(window: &winit::window::Window) -> bool {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return true;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return true;
    };
    let hwnd = handle.hwnd.get() as HWND;
    // SAFETY: two read-only window manager queries on the window this
    // thread created.
    unsafe { GetForegroundWindow() == hwnd && IsIconic(hwnd) == 0 }
}

#[cfg(not(windows))]
fn window_in_front(window: &winit::window::Window) -> bool {
    window.has_focus() && !window.is_minimized().unwrap_or(false)
}

/// Whether the flash asked for (the window behind) is done, by what the
/// window says of being minimized. macOS bounces the icon whenever
/// (LLWindowMacOSX::flashIcon). Linux flashes only a minimized window
/// (LLWindowSDL::flashIcon, indra/llwindow/llwindowsdl.cpp): on Wayland a
/// window cannot know it (`None`), so it never flashes there. Wayland's
/// attention request is an xdg_activation one, and a compositor that grants
/// it switches to the viewer's workspace and focuses it.
#[cfg(not(windows))]
fn flashes_here(minimized: Option<bool>) -> bool {
    cfg!(target_os = "macos") || minimized == Some(true)
}

#[cfg(windows)]
fn flash_window(window: &winit::window::Window) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FLASHW_TRAY, FLASHWINFO, FlashWindowEx};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let info = FLASHWINFO {
        cbSize: size_of::<FLASHWINFO>() as u32,
        hwnd: handle.hwnd.get() as HWND,
        dwFlags: FLASHW_TRAY,
        uCount: 10,
        dwTimeout: 500,
    };
    // SAFETY: a window manager call on the window this thread created,
    // with a filled FLASHWINFO of the right size.
    unsafe {
        FlashWindowEx(&info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_repeated_within_a_second_is_dropped() {
        let mut t = Throttle::default();
        let now = Instant::now();
        assert!(t.allow("Jane", now));
        assert!(!t.allow("Jane", now + Duration::from_millis(900)));
        // the drop started the second again
        assert!(!t.allow("Jane", now + Duration::from_millis(1800)));
        assert!(t.allow("Jane", now + Duration::from_millis(2900)));
        assert!(t.allow("Tom", now + Duration::from_millis(2900)));
    }

    #[test]
    fn the_flash_follows_its_settings() {
        let mut s = Settings::default();
        assert!(flashes(Cause::Message, &s) && flashes(Cause::Mention, &s) && flashes(Cause::ObjectIm, &s));
        assert!(!flashes(Cause::ScriptDialog, &s) && !flashes(Cause::Notice, &s) && !flashes(Cause::Keyword, &s));
        s.flash_on_message = false;
        assert!(!flashes(Cause::ObjectIm, &s) && !flashes(Cause::Mention, &s));
        s.flash_on_script_dialog = true;
        assert!(flashes(Cause::ScriptDialog, &s));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_flashes_only_a_minimized_window() {
        assert!(flashes_here(Some(true)));
        assert!(!flashes_here(Some(false)), "behind but not minimized");
        assert!(!flashes_here(None), "Wayland: never, its request would focus the window");
    }

    #[test]
    fn the_keyword_sound_needs_its_switch_and_the_interface_sounds() {
        let mut s = Settings::default();
        let mut k = KeywordSettings::default();
        assert_eq!(keyword_sound(&s, &k), None, "off by default, like PlayModeUISndFSKeywordSound");
        k.play_sound = true;
        assert_eq!(keyword_sound(&s, &k), Some(crate::keywords::DEFAULT_SOUND));
        s.audio.ui_sounds = false;
        assert_eq!(keyword_sound(&s, &k), None);
        s.audio.ui_sounds = true;
        k.sound = uuid::Uuid::nil();
        assert_eq!(keyword_sound(&s, &k), None);
    }

    #[test]
    fn the_system_hears_only_in_the_background_and_never_in_do_not_disturb() {
        let mut s = Settings::default();
        assert!(!notifies(&s, false, false, false), "off by default, like FSEnableGrowl");
        assert!(notifies(&s, true, false, false), "the demo turns them on");
        s.system_notifications = true;
        assert!(notifies(&s, false, false, false));
        assert!(!notifies(&s, false, true, false));
        assert!(!notifies(&s, false, false, true));
        s.system_notifications_when_active = true;
        assert!(notifies(&s, false, true, false));
        assert!(!notifies(&s, false, true, true));
    }

    #[test]
    fn the_body_is_one_line_cut_at_255_characters() {
        assert_eq!(body_text("a\n b\tc"), "a b c");
        let long = "é".repeat(300);
        let cut = body_text(&long);
        assert_eq!(cut.chars().count(), MAX_BODY + 1);
        assert!(cut.ends_with('…'));
    }
}
