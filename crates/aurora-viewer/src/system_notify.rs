//! System notifications: what the Legacy Viewer sent to Growl
//! (GrowlNotifierWin, growlmanager.cpp, originally LGPL 2.1) shown as
//! Windows notifications. A worker thread owns the WinRT objects, so the
//! frame never waits on the system; a click comes back as the notice's id.
//!
//! An application that is not installed from a package needs an
//! AppUserModelID known to Windows for its notifications to show: it is
//! written under `HKCU\Software\Classes\AppUserModelId` (its name and
//! icon), only once the user turns the notifications on.

use crossbeam_channel::{Receiver, Sender};

/// One notification to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    pub body: String,
}

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
pub fn toast_xml(title: &str, body: &str) -> String {
    let mut texts = format!("<text>{}</text>", escape(title));
    if !body.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape(body)));
    }
    format!(
        "<toast launch=\"aurora\"><visual><binding template=\"ToastGeneric\">{texts}</binding></visual><audio silent=\"true\"/></toast>"
    )
}

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

#[cfg(not(windows))]
fn run(rx: Receiver<Toast>, _clicks: Sender<u64>) {
    for toast in rx {
        log::info!("system notification (not shown on this system): {}", toast.title);
    }
}

#[cfg(windows)]
mod win {
    use super::{Receiver, Sender, Toast, toast_xml};
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

    /// The logo as a PNG file in the cache, for the registry (Windows reads
    /// the icon from a file). Its name follows the logo's size in bytes, so a
    /// new logo writes a new file instead of reusing the old one.
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

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
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
}
