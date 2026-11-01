//! Detect the dominant native axis of an assignment gesture.

use super::{AXES, Device, Settings, Snapshot};
use std::time::{Duration, Instant};

pub struct AxisCapture {
    device: String,
    initial: [f32; AXES],
    peaks: [f32; AXES],
    axis_count: usize,
    started: Option<Instant>,
}

impl AxisCapture {
    pub fn new(device: &Device) -> Self {
        Self {
            device: device.id.clone(),
            initial: device.axes,
            peaks: [0.0; AXES],
            axis_count: device.axis_count.min(AXES),
            started: None,
        }
    }

    pub fn poll(&mut self, snapshot: &Snapshot, settings: &Settings, now: Instant) -> Result<Option<i8>, ()> {
        let device = snapshot
            .selected(settings)
            .filter(|d| {
                d.id == self.device
                    && d.axis_count.min(AXES) == self.axis_count
                    && self.axis_count != 0
                    && snapshot
                        .updated
                        .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(250))
            })
            .ok_or(())?;
        let mut moving = 0.0f32;
        for (n, peak) in self.peaks.iter_mut().enumerate().take(self.axis_count) {
            let (value, initial) = (device.axes[n], self.initial[n]);
            if value.is_finite() && initial.is_finite() {
                let delta = (value.clamp(-1.0, 1.0) - initial.clamp(-1.0, 1.0)).abs();
                *peak = peak.max(delta);
                moving = moving.max(delta);
            }
        }
        let (mut strongest, mut second, mut selected) = (0.0f32, 0.0f32, 0);
        for (n, &peak) in self.peaks.iter().enumerate().take(self.axis_count) {
            if peak > strongest {
                second = strongest;
                strongest = peak;
                selected = n;
            } else {
                second = second.max(peak);
            }
        }
        let released = moving < 0.08;
        if strongest >= 0.25 {
            // A sideways motion can precede the intended vertical motion.
            // Observe the gesture before choosing; a flick ends on release.
            // Ambiguous diagonals need a clearer gesture after returning to rest.
            let started = *self.started.get_or_insert(now);
            if (released || now.saturating_duration_since(started) >= Duration::from_millis(180)) && strongest > second * 1.35 {
                return Ok(Some(selected as i8));
            }
        }
        if released {
            self.peaks.fill(0.0);
            self.started = None;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Snapshot, Settings, Instant) {
        let now = Instant::now();
        (
            Snapshot {
                devices: vec![Device::new("test".into(), "Test controller".into(), AXES)],
                updated: Some(now),
                ..Default::default()
            },
            Settings::default(),
            now,
        )
    }

    fn poll(capture: &mut AxisCapture, snapshot: &mut Snapshot, settings: &Settings, now: Instant) -> Result<Option<i8>, ()> {
        snapshot.updated = Some(now);
        capture.poll(snapshot, settings, now)
    }

    #[test]
    fn joystick_axis_capture_selects_the_dominant_gesture_instead_of_the_first_moving_axis() {
        let (mut snapshot, settings, now) = fixture();
        let mut capture = AxisCapture::new(&snapshot.devices[0]);
        snapshot.devices[0].axes[0] = 0.3;
        assert_eq!(poll(&mut capture, &mut snapshot, &settings, now), Ok(None));
        snapshot.devices[0].axes[1] = -0.85;
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(60)),
            Ok(None)
        );
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(200)),
            Ok(Some(1))
        );
    }

    #[test]
    fn joystick_axis_capture_ignores_resting_triggers_noise_and_ambiguous_diagonals() {
        let (mut snapshot, settings, now) = fixture();
        snapshot.devices[0].axes[6] = -1.0;
        let mut capture = AxisCapture::new(&snapshot.devices[0]);
        assert_eq!(poll(&mut capture, &mut snapshot, &settings, now), Ok(None));
        snapshot.devices[0].axes[0] = 0.02;
        snapshot.devices[0].axes[1] = -0.03;
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(100)),
            Ok(None)
        );
        snapshot.devices[0].axes[0] = 0.65;
        snapshot.devices[0].axes[1] = -0.66;
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(200)),
            Ok(None)
        );
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(400)),
            Ok(None)
        );
        snapshot.devices[0].axes[0] = 0.0;
        snapshot.devices[0].axes[1] = 0.0;
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(410)),
            Ok(None)
        );
        snapshot.devices[0].axes[6] = 1.0;
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(500)),
            Ok(None)
        );
        snapshot.devices[0].axes[6] = -1.0;
        assert_eq!(
            poll(&mut capture, &mut snapshot, &settings, now + Duration::from_millis(540)),
            Ok(Some(6))
        );
    }

    #[test]
    fn joystick_axis_capture_obeys_native_axis_count_and_rejects_stale_or_changed_devices() {
        let (mut snapshot, settings, now) = fixture();
        snapshot.devices[0].axis_count = 2;
        let mut capture = AxisCapture::new(&snapshot.devices[0]);
        snapshot.devices[0].axes[7] = 1.0;
        snapshot.devices[0].axes[0] = f32::NAN;
        snapshot.devices[0].axes[1] = f32::INFINITY;
        assert_eq!(poll(&mut capture, &mut snapshot, &settings, now), Ok(None));
        for scenario in ["changed", "disconnected", "stale", "count"] {
            let (mut snapshot, settings, now) = fixture();
            let mut capture = AxisCapture::new(&snapshot.devices[0]);
            match scenario {
                "changed" => snapshot.devices[0].id = "other".into(),
                "disconnected" => snapshot.devices.clear(),
                "stale" => snapshot.updated = Some(now - Duration::from_secs(1)),
                "count" => snapshot.devices[0].axis_count = 1,
                _ => unreachable!(),
            }
            assert_eq!(capture.poll(&snapshot, &settings, now), Err(()), "{scenario}");
        }
    }
}
