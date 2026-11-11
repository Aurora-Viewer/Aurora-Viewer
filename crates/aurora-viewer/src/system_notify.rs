//! System notifications: what the Legacy Viewer sent to Growl
//! (GrowlNotifierWin, DesktopNotifierLinux, growlmanager.cpp, originally
//! LGPL 2.1) shown as Windows notifications, or through the freedesktop
//! notification service on Linux. A worker thread owns the system objects,
//! so the frame never waits on the system; a click comes back as the
//! notice's id.
//!
//! An application that is not installed from a package needs an
//! AppUserModelID known to Windows for its notifications to show: it is
//! written under `HKCU\Software\Classes\AppUserModelId` (its name and
//! icon), only once the user turns the notifications on.
//!
//! Each notification says what kind it is and which conversation it
//! belongs to, so the desktop can group them: the freedesktop `category`
//! hint and an `x-aurora-thread` hint on Linux, the toast's `Group` on
//! Windows.

use crossbeam_channel::{Receiver, Sender};

/// One notification to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub kind: Kind,
    /// The conversation it belongs to (`thread_key`).
    pub thread: String,
    pub title: String,
    pub body: String,
}

/// What a notification is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A resident's instant message.
    Message,
    /// A line that mentions the agent (a group chat, a conference, nearby
    /// chat).
    Mention,
    /// An object's instant message.
    ObjectIm,
    /// A line that holds one of the keywords.
    Keyword,
    /// A script dialog or text box.
    ScriptDialog,
    /// An offer or a notice that came as a message (a teleport, a
    /// friendship, items, a group notice…).
    Offer,
    /// A message of the grid's system, shown in nearby chat.
    System,
    /// A friend came online.
    Online,
    /// A friend went offline.
    Offline,
    /// Anything else (a simulator alert, the region restart, a payment…).
    Notice,
}

/// The freedesktop category of a kind (Desktop Notifications
/// Specification, *Categories*): the standard name where one exists, else
/// the `x-vendor.class` form the specification asks for.
/// DesktopNotifierLinux::showNotification passes the Growl type's own name
/// there ("Friend logged off", growl_notifications.xml), which no desktop
/// knows.
pub fn category(kind: Kind) -> &'static str {
    match kind {
        Kind::Message => "im.received",
        Kind::Online => "presence.online",
        Kind::Offline => "presence.offline",
        Kind::Mention => "x-aurora.mention",
        Kind::ObjectIm => "x-aurora.object-im",
        Kind::Keyword => "x-aurora.keyword",
        Kind::ScriptDialog => "x-aurora.script-dialog",
        Kind::Offer => "x-aurora.offer",
        Kind::System => "x-aurora.system",
        Kind::Notice => "x-aurora.notice",
    }
}

/// The thread every friend's online and offline notice shares.
pub const PRESENCE_THREAD: &str = "presence";

/// The worker thread: notifications go in, clicks come out.
pub struct Notifier {
    tx: Sender<Toast>,
    clicks: Receiver<u64>,
}

impl Notifier {
    pub fn start() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Toast>();
        let (click_tx, clicks) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("system-notify".into())
            .spawn(move || run(rx, click_tx));
        if let Err(e) = spawned {
            log::warn!("system notifications: no worker thread ({e})");
        }
        Self { tx, clicks }
    }

    pub fn show(&self, toast: Toast) {
        // the worker gone (no thread, no notifier): the notice is lost
        let _ = self.tx.send(toast);
    }

    /// The id of a notification the user clicked since the last call.
    pub fn clicked(&self) -> Option<u64> {
        self.clicks.try_recv().ok()
    }
}

/// The `<toast>` of a notification (ToastGeneric: the title in bold, the
/// body below), silent: Aurora plays its own sounds.
#[cfg(any(windows, test))]
pub fn toast_xml(title: &str, body: &str) -> String {
    let mut texts = format!("<text>{}</text>", escape(title));
    if !body.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape(body)));
    }
    format!(
        "<toast launch=\"aurora\"><visual><binding template=\"ToastGeneric\">{texts}</binding></visual><audio silent=\"true\"/></toast>"
    )
}

/// A Windows toast's Tag and Group: its id, its thread, each cut to the
/// 64 characters Windows takes (a session's id has 36).
#[cfg(any(windows, test))]
fn toast_tag_group(toast: &Toast) -> (String, String) {
    const MAX: usize = 64;
    let cut = |s: &str| s.chars().take(MAX).collect::<String>();
    (toast.id.to_string(), cut(&toast.thread))
}

#[cfg(any(windows, test))]
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // characters XML 1.0 refuses (a control code in a chat line)
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(windows)]
fn run(rx: Receiver<Toast>, clicks: Sender<u64>) {
    win::run(rx, clicks);
}

#[cfg(target_os = "linux")]
fn run(rx: Receiver<Toast>, clicks: Sender<u64>) {
    freedesktop::run(rx, clicks);
}

#[cfg(not(any(windows, target_os = "linux")))]
fn run(rx: Receiver<Toast>, _clicks: Sender<u64>) {
    for toast in rx {
        log::info!("system notification (not shown on this system): {}", toast.title);
    }
}

/// The logo as a PNG file in the cache: Windows' registry and the
/// freedesktop service both read the icon from a file. Its name follows the
/// logo's size in bytes, so a new logo writes a new file instead of reusing
/// the old one.
#[cfg(any(windows, target_os = "linux"))]
fn icon_file() -> Option<std::path::PathBuf> {
    let dir = crate::settings::cache_dir();
    let name = format!("notification-icon-{}.png", crate::app::LOGO_PNG.len());
    let path = dir.join(&name);
    if path.exists() {
        return Some(path);
    }
    // the icons of earlier logos
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let old = entry.file_name();
            let old = old.to_string_lossy();
            if old.starts_with("notification-icon") && old.ends_with(".png") && old != name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let img = match image::load_from_memory(crate::app::LOGO_PNG) {
        Ok(img) => img.resize(256, 256, image::imageops::FilterType::Triangle),
        Err(e) => {
            log::warn!("system notifications: logo not decoded ({e})");
            return None;
        }
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("system notifications: no cache folder ({e})");
        return None;
    }
    match img.save(&path) {
        Ok(()) => Some(path),
        Err(e) => {
            log::warn!("system notifications: icon not written ({e})");
            None
        }
    }
}

/// Markup for a server that reads the body as such (the `body-markup`
/// capability): only `&`, `<` and `>` are special there, and a control code
/// would break the server's parser as it breaks XML.
#[cfg(any(target_os = "linux", test))]
fn markup_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

/// The freedesktop notification service (`org.freedesktop.Notifications`,
/// Desktop Notifications Specification 1.2) over the session bus: the
/// D-Bus side of what libnotify does for DesktopNotifierLinux, without
/// loading libnotify. Each notice is its own notification, as on Windows
/// (the Legacy Viewer updates a single one, so a new notice replaced the
/// last); a click on it (the `default` action) comes back as its id.
#[cfg(target_os = "linux")]
mod freedesktop {
    use super::{Receiver, Sender, Toast, icon_file, markup_escape};
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex, PoisonError};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::Value;

    const DEST: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";
    /// The application name the server groups the notices by.
    const APP_NAME: &str = "Aurora Viewer";
    /// The desktop entry's id (`aurora-viewer.desktop`), from which a server
    /// takes the name and icon it shows and its per-application rules.
    const DESKTOP_ENTRY: &str = "aurora-viewer";
    /// Notices whose click is still awaited: the server's id, then ours. A
    /// server tells when one closes; the bound covers one that does not.
    const KEEP: usize = 32;

    type Pending = Arc<Mutex<VecDeque<(u32, u64)>>>;

    pub(super) fn run(rx: Receiver<Toast>, clicks: Sender<u64>) {
        let proxy = match Connection::session().and_then(|bus| Proxy::new_owned(bus, DEST, PATH, DEST)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("system notifications: no session bus ({e})");
                return;
            }
        };
        // Read once for the session: a server started (or replaced by one
        // with `actions`) after the viewer gets no click handling until the
        // next start.
        let caps: Vec<String> = proxy.call("GetCapabilities", &()).unwrap_or_else(|e| {
            log::warn!("system notifications: no notification service ({e})");
            Vec::new()
        });
        if let Ok((name, vendor, version, spec)) = proxy.call::<_, _, (String, String, String, String)>("GetServerInformation", &()) {
            log::info!("system notifications: {name} {version} ({vendor}, spec {spec}), capabilities {caps:?}");
        }
        let markup = caps.iter().any(|c| c == "body-markup");
        let actions = caps.iter().any(|c| c == "actions");
        let pending: Pending = Arc::default();
        if actions {
            listen(&proxy, pending.clone(), clicks);
        }
        let icon = icon_file().map(|p| p.display().to_string()).unwrap_or_default();
        for toast in rx {
            let body = if markup { markup_escape(&toast.body) } else { toast.body.clone() };
            // a server that clicks without a label still needs the pair
            let open = aurora_i18n::tr!("system-notification-open");
            let action_list: Vec<&str> = if actions { vec!["default", open] } else { Vec::new() };
            let hints = hints(&toast);
            // expire_timeout -1: the server's own delay, as on Windows
            let args = (
                APP_NAME,
                0u32,
                icon.as_str(),
                toast.title.as_str(),
                body.as_str(),
                action_list,
                hints,
                -1i32,
            );
            match proxy.call::<_, _, u32>("Notify", &args) {
                Ok(server_id) if actions => {
                    let mut p = pending.lock().unwrap_or_else(PoisonError::into_inner);
                    p.push_back((server_id, toast.id));
                    if p.len() > KEEP {
                        p.pop_front();
                    }
                }
                Ok(_) => {}
                Err(e) => log::warn!("system notification not shown ({e})"),
            }
        }
    }

    /// Aurora plays its own sounds; the urgency is the Legacy Viewer's
    /// (NOTIFY_URGENCY_NORMAL). The kind and the thread let the desktop
    /// group the notices; none replaces another (`replaces_id` 0).
    fn hints(toast: &Toast) -> HashMap<&'static str, Value<'_>> {
        HashMap::from([
            ("desktop-entry", Value::from(DESKTOP_ENTRY)),
            ("suppress-sound", Value::from(true)),
            ("urgency", Value::from(1u8)),
            ("category", Value::from(super::category(toast.kind))),
            ("x-aurora-thread", Value::from(toast.thread.as_str())),
        ])
    }

    /// A thread that waits for the server's signals: a click sends our id,
    /// a close forgets the notice. The subscription is made before the
    /// first notice, so no click is missed.
    fn listen(proxy: &Proxy<'static>, pending: Pending, clicks: Sender<u64>) {
        let signals = match proxy.receive_all_signals() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("system notifications: clicks not followed ({e})");
                return;
            }
        };
        let spawned = std::thread::Builder::new().name("system-notify-clicks".into()).spawn(move || {
            for msg in signals {
                let header = msg.header();
                let Some(member) = header.member() else { continue };
                let body = msg.body();
                let (server_id, clicked) = match member.as_str() {
                    "ActionInvoked" => match body.deserialize::<(u32, String)>() {
                        Ok((id, action)) => (id, action == "default"),
                        Err(_) => continue,
                    },
                    "NotificationClosed" => match body.deserialize::<(u32, u32)>() {
                        Ok((id, _reason)) => (id, false),
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                let mut p = pending.lock().unwrap_or_else(PoisonError::into_inner);
                // the newest first: a restarted server numbers from 1 again,
                // and the dead one's notices never close
                let Some(at) = p.iter().rposition(|&(s, _)| s == server_id) else {
                    continue;
                };
                if clicked {
                    // a click closes the notice too: the close finds nothing
                    let Some((_, id)) = p.remove(at) else { continue };
                    drop(p);
                    if clicks.send(id).is_err() {
                        return;
                    }
                } else {
                    p.remove(at);
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("system notifications: no click thread ({e})");
        }
    }
}

#[cfg(windows)]
mod win {
    use super::{Receiver, Sender, Toast, icon_file, toast_xml};
    use std::collections::VecDeque;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastFailedEventArgs, ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::{HSTRING, IInspectable, PCWSTR, Ref};

    /// The application's identity for Windows' notifications.
    const AUMID: &str = "Aurora.Viewer";
    /// Notifications kept alive for their click handler.
    const KEEP: usize = 32;

    pub(super) fn run(rx: Receiver<Toast>, clicks: Sender<u64>) {
        // SAFETY: called once at the start of this thread, which owns its
        // apartment; WinRT stays initialized until the thread ends.
        if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            log::warn!("system notifications: RoInitialize failed ({e})");
            return;
        }
        register();
        let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("system notifications: no notifier ({e})");
                return;
            }
        };
        let mut shown: VecDeque<ToastNotification> = VecDeque::new();
        for toast in rx {
            match build(&toast, &clicks).and_then(|t| notifier.Show(&t).map(|()| t)) {
                Ok(t) => {
                    shown.push_back(t);
                    if shown.len() > KEEP {
                        shown.pop_front();
                    }
                }
                Err(e) => log::warn!("system notification not shown ({e})"),
            }
        }
    }

    fn build(toast: &Toast, clicks: &Sender<u64>) -> windows::core::Result<ToastNotification> {
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(toast_xml(&toast.title, &toast.body)))?;
        let t = ToastNotification::CreateToastNotification(&xml)?;
        // the Group gathers a conversation's notices; the Tag, unique,
        // keeps each one (a same Tag and Group replaces)
        let (tag, group) = super::toast_tag_group(toast);
        t.SetTag(&HSTRING::from(tag))?;
        t.SetGroup(&HSTRING::from(group))?;
        let (id, clicks) = (toast.id, clicks.clone());
        t.Activated(&TypedEventHandler::new(move |_: Ref<ToastNotification>, _: Ref<IInspectable>| {
            let _ = clicks.send(id);
            Ok(())
        }))?;
        t.Failed(&TypedEventHandler::new(
            |_: Ref<ToastNotification>, args: Ref<ToastFailedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    log::warn!("system notification failed ({:?})", args.ErrorCode());
                }
                Ok(())
            },
        ))?;
        Ok(t)
    }

    /// The AppUserModelID's name and icon (what Windows shows above the
    /// notification), written for the current user.
    fn register() {
        let key = format!("Software\\Classes\\AppUserModelId\\{AUMID}");
        let icon = icon_file();
        let mut values = vec![("DisplayName", "Aurora Viewer".to_owned())];
        if let Some(icon) = icon {
            values.push(("IconUri", icon.display().to_string()));
        }
        for (field, value) in values {
            let key = wide(&key);
            let name = wide(field);
            let data = wide(&value);
            // SAFETY: the three buffers are NUL-terminated UTF-16 that live
            // across the call; the size given is the data's, in bytes.
            let err = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(name.as_ptr()),
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    (data.len() * 2) as u32,
                )
            };
            if err.is_err() {
                log::warn!("system notifications: {field} not registered ({err:?})");
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn app_id_key() -> String {
        format!(r"Software\Classes\AppUserModelId\{AUMID}")
    }

    /// The AppUserModelID's key, when this user's registry holds it (the
    /// notifications were turned on once).
    pub fn app_id_registration() -> Option<String> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW};
        let key = wide(&app_id_key());
        let mut opened: HKEY = std::ptr::null_mut();
        // SAFETY: a NUL-terminated string alive for the call and a handle
        // to fill, closed below when the call opened it.
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut opened) };
        if status != ERROR_SUCCESS {
            return None;
        }
        // SAFETY: the handle the call above opened.
        unsafe { RegCloseKey(opened) };
        Some(format!(r"HKEY_CURRENT_USER\{}", app_id_key()))
    }

    /// Removes the AppUserModelID's key; the notifications write it again
    /// when they start.
    pub fn unregister_app_id() -> Result<(), String> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
        let key = wide(&app_id_key());
        // SAFETY: a NUL-terminated string alive for the call.
        match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) } {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            status => Err(format!("{}: error {status}", app_id_key())),
        }
    }
}

#[cfg(windows)]
pub use win::{app_id_registration, unregister_app_id};

/// Only Windows needs a registered identity for its notifications.
#[cfg(not(windows))]
pub fn app_id_registration() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn unregister_app_id() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toast_is_escaped_and_silent() {
        let xml = toast_xml("Jane <Doe>", "a & b \"c\" 'd'\u{1}");
        assert_eq!(
            xml,
            "<toast launch=\"aurora\"><visual><binding template=\"ToastGeneric\"><text>Jane &lt;Doe&gt;</text>\
             <text>a &amp; b &quot;c&quot; &apos;d&apos;</text></binding></visual><audio silent=\"true\"/></toast>"
        );
        // a title alone has a single line
        assert!(!toast_xml("Jane is online.", "").contains("<text></text>"));
    }

    #[test]
    fn each_kind_has_its_freedesktop_category() {
        // the specification's own names where it has one
        assert_eq!(category(Kind::Message), "im.received");
        assert_eq!(category(Kind::Online), "presence.online");
        assert_eq!(category(Kind::Offline), "presence.offline");
        // the rest in the x-vendor form, each its own
        let others = [
            Kind::Mention,
            Kind::ObjectIm,
            Kind::Keyword,
            Kind::ScriptDialog,
            Kind::Offer,
            Kind::System,
            Kind::Notice,
        ];
        let names: std::collections::HashSet<_> = others.iter().map(|k| category(*k)).collect();
        assert_eq!(names.len(), others.len());
        assert!(names.iter().all(|n| n.starts_with("x-aurora.") && !n.contains(' ')));
    }

    #[test]
    fn the_windows_tag_is_the_id_and_the_group_the_thread() {
        let toast = |thread: &str| Toast {
            id: 42,
            kind: Kind::Message,
            thread: thread.into(),
            title: "Jane".into(),
            body: String::new(),
        };
        let session = "6f1e3b0a-1c1d-4a3e-9b1a-2b3c4d5e6f70";
        assert_eq!(toast_tag_group(&toast(session)), ("42".into(), session.into()));
        // never past Windows' 64 characters
        let (_, group) = toast_tag_group(&toast(&"x".repeat(80)));
        assert_eq!(group.chars().count(), 64);
    }

    #[test]
    fn the_markup_body_is_escaped() {
        assert_eq!(
            markup_escape("<b>a & b</b> \"c\" 'd'\u{1}\n"),
            "&lt;b&gt;a &amp; b&lt;/b&gt; \"c\" 'd'\n"
        );
    }
}
