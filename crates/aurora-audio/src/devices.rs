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

/// Names of the audio output devices of the default host. Empty when none can be listed. Enumeration talks to the OS:
/// call it when a settings panel opens, not every frame.
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
/// so the stream stays there when the desktop default changes; ALSA's
/// "default" and the Legacy Viewer follow the new default, so the output and
/// capture threads reopen their stream when this reports a change.
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
