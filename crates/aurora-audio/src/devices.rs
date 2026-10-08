//! Device enumeration and lookup by name (cpal default host).
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

/// Names of the audio output devices of the default host (WASAPI on
/// Windows). Empty when none can be listed. Enumeration talks to the OS:
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
