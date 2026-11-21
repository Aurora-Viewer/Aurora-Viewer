//! Device enumeration and lookup by name (cpal default host: WASAPI on
//! Windows; on Linux the PulseAudio protocol, which PipeWire also serves,
//! else ALSA).
//!
//! Devices are identified by their human-readable name, as Firestorm does
//! for its `VoiceOutputAudioDevice` / `VoiceInputAudioDevice` settings.
//! When two devices share a name the first one listed by the host is used.

use cpal::traits::{DeviceTrait, HostTrait};

/// Human-readable name of a device.
pub(crate) fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Names of the audio output devices of the default host. Empty when none
/// can be listed. Enumeration talks to the OS: call it when a settings
/// panel opens, not every frame.
pub fn output_devices() -> Vec<String> {
    match cpal::default_host().output_devices() {
        Ok(list) => list.map(|d| device_name(&d)).collect(),
        Err(e) => {
            log::debug!("cannot list audio output devices: {e}");
            Vec::new()
        }
    }
}

/// Names of the audio input (microphone) devices of the default host.
/// Empty when none can be listed.
pub fn input_devices() -> Vec<String> {
    match cpal::default_host().input_devices() {
        Ok(list) => list.map(|d| device_name(&d)).collect(),
        Err(e) => {
            log::debug!("cannot list audio input devices: {e}");
            Vec::new()
        }
    }
}

/// The output and input device names, listed together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceLists {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
}

impl DeviceLists {
    /// Both lists, now. Talks to the OS and can block for a long time (a
    /// Bluetooth headset waking up, HDMI audio, a stuck virtual driver: 28 s
    /// in #2867): call it off the UI thread, through [`DeviceLister`].
    pub fn list() -> Self {
        Self {
            outputs: output_devices(),
            inputs: input_devices(),
        }
    }
}

/// Lists the devices on a background thread, so that a slow answer from
/// the OS never holds a frame. The caller keeps showing its last lists until
/// [`poll`](Self::poll) hands out new ones.
pub struct DeviceLister {
    list: fn() -> DeviceLists,
    /// The running listing and when it started.
    pending: Option<(crossbeam_channel::Receiver<DeviceLists>, std::time::Instant)>,
    /// Asked again while a listing ran: list once more after it, so that a
    /// device plugged in meanwhile is not missed.
    again: bool,
    /// A listing unanswered for this long is given up at the next request.
    give_up: std::time::Duration,
}

impl Default for DeviceLister {
    fn default() -> Self {
        Self::with(DeviceLists::list, std::time::Duration::from_secs(10))
    }
}

impl DeviceLister {
    fn with(list: fn() -> DeviceLists, give_up: std::time::Duration) -> Self {
        Self {
            list,
            pending: None,
            again: false,
            give_up,
        }
    }

    /// Start a listing; while one runs, another follows it. A listing the OS
    /// has held too long (10 s) is abandoned (its thread stays
    /// blocked in the OS, its answer is dropped) and a fresh one starts.
    /// Never blocks.
    pub fn request(&mut self) {
        if let Some((_, started)) = &self.pending {
            if started.elapsed() < self.give_up {
                self.again = true;
                return;
            }
            log::warn!("audio device listing unanswered for {:?}; listing again", started.elapsed());
            self.pending = None;
        }
        self.again = false;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let list = self.list;
        let spawned = std::thread::Builder::new().name("audio devices".into()).spawn(move || {
            let _ = tx.send(list());
        });
        match spawned {
            Ok(_) => self.pending = Some((rx, std::time::Instant::now())),
            Err(e) => log::warn!("cannot start listing the audio devices: {e}"),
        }
    }

    /// Whether a listing is running.
    pub fn listing(&self) -> bool {
        self.pending.is_some()
    }

    /// The lists of a finished listing, once. Never blocks.
    pub fn poll(&mut self) -> Option<DeviceLists> {
        let result = match self.pending.as_ref()?.0.try_recv() {
            Ok(lists) => Some(lists),
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                log::warn!("the audio device listing stopped without an answer");
                None
            }
        };
        self.pending = None;
        if std::mem::take(&mut self.again) {
            self.request();
        }
        result
    }
}

/// Whether `device` is a sink or source of a PulseAudio server (PipeWire
/// included, through pipewire-pulse). Without an explicit buffer size such a
/// stream gets the server's default, two seconds of latency.
pub(crate) fn is_pulse(device: &cpal::Device) -> bool {
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd"))]
    {
        device.id().is_ok_and(|id| id.host() == cpal::HostId::PulseAudio)
    }
    #[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd")))]
    {
        let _ = device;
        false
    }
}

/// Watches the sound server's default device while "Default" is in use. The
/// PulseAudio host opens a stream on the sink or source it resolved by index,
/// so the stream stays there when the desktop default changes. ALSA's
/// "default" follows the new default, and so does the Legacy Viewer's voice
/// (`LLWebRTCImpl::updateDevices`, indra/llwebrtc/llwebrtc.cpp), so the
/// output and capture threads reopen their stream when this reports a change.
pub(crate) struct DefaultWatch {
    host: cpal::Host,
    direction: Direction,
    current: cpal::DeviceId,
}

impl DefaultWatch {
    /// How often the threads check (one round trip to the server, off the
    /// audio callback).
    pub(crate) const EVERY: std::time::Duration = std::time::Duration::from_secs(2);

    /// A watch on `opened`, the default device just opened; `None` off the
    /// PulseAudio host, whose other hosts need none.
    pub(crate) fn new(direction: Direction, opened: &cpal::Device) -> Option<Self> {
        if !is_pulse(opened) {
            return None;
        }
        let current = opened.id().ok()?;
        Some(Self {
            host: cpal::default_host(),
            direction,
            current,
        })
    }

    /// True when the server's default is no longer the opened device. A
    /// failed query (server restarting) reports no change: the stream's own
    /// error reopens it.
    pub(crate) fn changed(&self) -> bool {
        let default = match self.direction {
            Direction::Output => self.host.default_output_device(),
            Direction::Input => self.host.default_input_device(),
        };
        default.and_then(|d| d.id().ok()).is_some_and(|id| id != self.current)
    }
}

/// Which way a device is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Output,
    Input,
}

/// A device picked for a request.
pub(crate) struct Picked {
    pub device: cpal::Device,
    /// A named device was requested but is missing: this is the default.
    pub fallback: bool,
}

/// Finds the device called `name`, or the system default (`None`, or when
/// the named device is missing; that case is logged).
pub(crate) fn pick(dir: Direction, name: Option<&str>) -> Option<Picked> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let found = match dir {
            Direction::Output => host.output_devices().ok().and_then(|mut it| it.find(|d| device_name(d) == name)),
            Direction::Input => host.input_devices().ok().and_then(|mut it| it.find(|d| device_name(d) == name)),
        };
        if let Some(device) = found {
            return Some(Picked { device, fallback: false });
        }
        log::warn!("audio {dir:?} device {name:?} not found; using the system default");
    }
    let device = match dir {
        Direction::Output => host.default_output_device(),
        Direction::Input => host.default_input_device(),
    }?;
    Some(Picked {
        device,
        fallback: name.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn speakers() -> DeviceLists {
        DeviceLists {
            outputs: vec!["Speakers".into()],
            inputs: vec!["Microphone".into()],
        }
    }

    /// Waits for the lists, failing after `limit`.
    fn wait(lister: &mut DeviceLister, limit: Duration) -> DeviceLists {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(lists) = lister.poll() {
                return lists;
            }
            assert!(Instant::now() < deadline, "no device lists after {limit:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_slow_device_listing_never_blocks_the_caller() {
        // as the OS's audio service answering late (#2867)
        fn slow() -> DeviceLists {
            std::thread::sleep(Duration::from_millis(600));
            speakers()
        }
        let mut lister = DeviceLister::with(slow, Duration::from_secs(60));
        assert!(!lister.listing());
        assert_eq!(lister.poll(), None);
        let started = Instant::now();
        lister.request();
        assert_eq!(lister.poll(), None);
        assert!(lister.listing());
        // a frame's worth: the request and the poll return at once
        assert!(started.elapsed() < Duration::from_millis(300), "{:?}", started.elapsed());
        assert_eq!(wait(&mut lister, Duration::from_secs(30)), speakers());
        assert!(!lister.listing());
        assert_eq!(lister.poll(), None, "handed out once");
    }

    #[test]
    fn a_request_during_a_listing_lists_once_more_after_it() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn counted() -> DeviceLists {
            let n = CALLS.fetch_add(1, Ordering::SeqCst) + 1;
            std::thread::sleep(Duration::from_millis(100));
            DeviceLists {
                outputs: vec![format!("listing {n}")],
                inputs: Vec::new(),
            }
        }
        let mut lister = DeviceLister::with(counted, Duration::from_secs(60));
        lister.request();
        // a device plugged in meanwhile: asked three more times, listed once more
        lister.request();
        lister.request();
        lister.request();
        assert_eq!(wait(&mut lister, Duration::from_secs(30)).outputs, ["listing 1"]);
        assert!(lister.listing(), "the second listing follows");
        assert_eq!(wait(&mut lister, Duration::from_secs(30)).outputs, ["listing 2"]);
        assert!(!lister.listing());
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_listing_that_dies_ends_without_lists() {
        fn broken() -> DeviceLists {
            panic!("driver failure");
        }
        let mut lister = DeviceLister::with(broken, Duration::from_secs(60));
        lister.request();
        let deadline = Instant::now() + Duration::from_secs(30);
        while lister.listing() {
            assert_eq!(lister.poll(), None);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_hung_listing_is_replaced_at_the_next_request() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn first_hangs() -> DeviceLists {
            if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_secs(3));
                return DeviceLists {
                    outputs: vec!["stale".into()],
                    inputs: Vec::new(),
                };
            }
            speakers()
        }
        let mut lister = DeviceLister::with(first_hangs, Duration::from_millis(100));
        lister.request();
        // asked again before the limit: it waits for the running one
        lister.request();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "no second listing before the limit");
        lister.request();
        assert_eq!(wait(&mut lister, Duration::from_secs(2)), speakers());
        assert!(!lister.listing(), "the earlier request was served by the fresh listing");
        std::thread::sleep(Duration::from_secs(3));
        assert_eq!(lister.poll(), None, "the stale answer is dropped");
    }
}
