//! Fixed-size marker metadata and closed-vocabulary public report schema.

#[cfg(any(not(target_os = "macos"), test))]
pub(super) const SLOTS: usize = 64;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const WORDS: usize = 64;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const PAYLOAD_WORDS: usize = SLOTS * WORDS;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const LAST_EPOCH: u32 = u32::MAX / WORDS as u32 - 1;

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    #[default]
    Auto,
    Transfer,
}
#[cfg(any(not(target_os = "macos"), test))]
impl Mode {
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn environment() -> Self {
        match std::env::var("AURORA_GPU_BREADCRUMBS").as_deref() {
            Ok("off") => Self::Off,
            Ok("transfer") => Self::Transfer,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Method {
    #[default]
    Unavailable,
    Amd,
    Nv,
    Transfer,
}
impl Method {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Amd => "AMD buffer marker",
            Self::Nv => "NV checkpoint",
            Self::Transfer => "transfer (deep diagnostics)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub(super) amd: bool,
    pub(super) nv: bool,
    pub(super) fault: bool,
    pub(super) coherent: bool,
}
#[cfg(any(not(target_os = "macos"), test))]
impl Capabilities {
    pub(super) fn select(self, mode: Mode) -> Method {
        match mode {
            Mode::Off => Method::Unavailable,
            Mode::Transfer => Method::Transfer,
            Mode::Auto if self.amd => Method::Amd,
            Mode::Auto if self.nv => Method::Nv,
            Mode::Auto => Method::Unavailable,
        }
    }
}

/// Closed dictionary: no scene/object/user labels enter the public report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Boundary {
    #[default]
    FrameBegin,
    ShadowsBegin,
    ShadowsEnd,
    DepthBegin,
    DepthEnd,
    GtaoBegin,
    GtaoEnd,
    ContactBegin,
    ContactEnd,
    ReflectionsBegin,
    ReflectionsEnd,
    ProbesBegin,
    ProbesEnd,
    SceneBegin,
    SceneEnd,
    SsrWaterBlendBegin,
    SsrWaterBlendEnd,
    PostBegin,
    PostEnd,
    FrameEnd,
    PhotoBegin,
    PhotoEnd,
}
impl Boundary {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::FrameBegin => "frame begin",
            Self::FrameEnd => "frame end",
            Self::ShadowsBegin => "shadows begin",
            Self::ShadowsEnd => "shadows end",
            Self::DepthBegin => "depth begin",
            Self::DepthEnd => "depth end",
            Self::GtaoBegin => "AO (GTAO/SSAO) begin",
            Self::GtaoEnd => "AO (GTAO/SSAO) end",
            Self::ContactBegin => "contact shadows begin",
            Self::ContactEnd => "contact shadows end",
            Self::ReflectionsBegin => "reflections begin",
            Self::ReflectionsEnd => "reflections end",
            Self::ProbesBegin => "probes begin",
            Self::ProbesEnd => "probes end",
            Self::SceneBegin => "scene begin",
            Self::SceneEnd => "scene end",
            Self::SsrWaterBlendBegin => "scene-b (SSR/water/blend) begin",
            Self::SsrWaterBlendEnd => "scene-b (SSR/water/blend) end",
            Self::PostBegin => "post begin",
            Self::PostEnd => "post end",
            Self::PhotoBegin => "photo begin",
            Self::PhotoEnd => "photo end",
        }
    }
    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn begin(self) -> bool {
        matches!(
            self,
            Self::FrameBegin
                | Self::ShadowsBegin
                | Self::DepthBegin
                | Self::GtaoBegin
                | Self::ContactBegin
                | Self::ReflectionsBegin
                | Self::ProbesBegin
                | Self::SceneBegin
                | Self::SsrWaterBlendBegin
                | Self::PostBegin
                | Self::PhotoBegin
        )
    }
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug)]
pub(super) struct Slot {
    pub(super) epoch: u32,
    pub(super) frame: u64,
    pub(super) pending: bool,
    pub(super) count: u8,
    pub(super) labels: [Boundary; WORDS],
}
#[cfg(any(not(target_os = "macos"), test))]
impl Default for Slot {
    fn default() -> Self {
        Self {
            epoch: 0,
            frame: 0,
            pending: false,
            count: 0,
            labels: [Boundary::FrameBegin; WORDS],
        }
    }
}
#[cfg(any(not(target_os = "macos"), test))]
#[derive(Debug)]
pub(super) struct Ring {
    pub(super) generation: u64,
    pub(super) epoch: u32,
    pub(super) slots: [Slot; SLOTS],
    pub(super) saturated: bool,
}
#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    pub(super) generation: u64,
    pub(super) epoch: u32,
    pub(super) slot: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Observed {
    pub(super) frame: u64,
    pub(super) boundary: Boundary,
    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) token: u32,
}
#[cfg(any(not(target_os = "macos"), test))]
impl Ring {
    pub(super) fn new(generation: u64) -> Self {
        Self {
            generation,
            epoch: 0,
            slots: [Slot::default(); SLOTS],
            saturated: false,
        }
    }
    pub(super) fn reserve(&mut self, frame: u64) -> Option<Ticket> {
        let candidate = (self.epoch as usize..self.epoch as usize + SLOTS)
            .map(|n| n % SLOTS)
            .find(|&n| !self.slots[n].pending);
        let Some(slot) = candidate.filter(|_| self.epoch < LAST_EPOCH) else {
            self.saturated = true;
            return None;
        };
        self.epoch += 1;
        self.slots[slot] = Slot {
            epoch: self.epoch,
            frame,
            pending: true,
            ..Slot::default()
        };
        Some(Ticket {
            generation: self.generation,
            epoch: self.epoch,
            slot,
        })
    }
    pub(super) fn current(&mut self, ticket: Ticket) -> Option<&mut Slot> {
        if ticket.generation != self.generation {
            return None;
        }
        self.slots
            .get_mut(ticket.slot)
            .filter(|slot| slot.epoch == ticket.epoch && slot.pending)
    }
    pub(super) fn record(&mut self, ticket: Ticket, boundary: Boundary) -> Option<(usize, u32)> {
        let slot = self.current(ticket)?;
        let word = usize::from(slot.count);
        if word == WORDS {
            return None;
        }
        slot.labels[word] = boundary;
        slot.count += 1;
        Some((ticket.slot * WORDS + word, ticket.epoch * WORDS as u32 + word as u32))
    }
    pub(super) fn complete(&mut self, ticket: Ticket) {
        if let Some(slot) = self.current(ticket) {
            slot.pending = false;
        }
    }
    pub(super) fn decode(&self, generation: u64, offset: usize, token: u32) -> Option<Observed> {
        if generation != self.generation || offset >= PAYLOAD_WORDS {
            return None;
        }
        let slot = &self.slots[offset / WORDS];
        let word = offset % WORDS;
        (word < usize::from(slot.count) && token == slot.epoch * WORDS as u32 + word as u32).then(|| Observed {
            frame: slot.frame,
            boundary: slot.labels[word],
            token,
        })
    }
    pub(super) fn checkpoint(&self, token: u32) -> Option<Observed> {
        let epoch = token / WORDS as u32;
        let slot = self.slots.iter().position(|slot| slot.epoch == epoch)?;
        self.decode(self.generation, slot * WORDS + token as usize % WORDS, token)
    }
}

/// All public values are generated from enums, booleans and bounded counts.
/// Private driver strings/addresses are kept in a separate artifact.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub(super) offered: Capabilities,
    pub(super) method: Method,
    pub(super) fault_enabled: bool,
    pub(super) generation: u64,
    pub(super) observed: Option<Observed>,
    pub(super) stage: Stage,
    pub(super) saturated: bool,
    pub(super) fault: FaultStatus,
    pub(super) addresses: u32,
    pub(super) vendors: u32,
    pub(super) address_kinds: u32,
}
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Stage {
    #[default]
    Unavailable,
    Top,
    Bottom,
    Other,
    Transfer,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FaultStatus {
    #[default]
    Unsupported,
    Disabled,
    Pending,
    NoDetail,
    Failed,
    Truncated,
    Collected,
    Simulated,
    Unconfirmed,
    NotRequested,
}
impl Report {
    pub fn breadcrumbs(&self) -> String {
        let yes = |b| if b { "yes" } else { "no" };
        let stage = match self.stage {
            Stage::Unavailable => "unavailable",
            Stage::Top => "top",
            Stage::Bottom => "bottom",
            Stage::Other => "other",
            Stage::Transfer => "transfer",
        };
        let boundary = self
            .observed
            .map_or_else(|| "unavailable".into(), |o| format!("{} (frame {})", o.boundary.name(), o.frame));
        format!(
            "offered AMD {}, NV {}; enabled {}; generation {}; last observed boundary {boundary}, stage {stage}; best-effort marker snapshot{}",
            yes(self.offered.amd),
            yes(self.offered.nv),
            self.method.name(),
            self.generation,
            if self.saturated { "; ring saturated" } else { "" }
        )
    }
    pub fn fault(&self) -> String {
        let status = match self.fault {
            FaultStatus::Unsupported => "unsupported",
            FaultStatus::Disabled => "supported-but-disabled",
            FaultStatus::Pending => "not collected within budget",
            FaultStatus::NoDetail => "no detail",
            FaultStatus::Failed => "query failed",
            FaultStatus::Truncated => "truncated",
            FaultStatus::Collected => "collected",
            FaultStatus::Simulated => "not queried (simulated loss)",
            FaultStatus::Unconfirmed => "not queried (Vulkan loss unconfirmed)",
            FaultStatus::NotRequested => "not requested (no confirmed loss)",
        };
        let kinds: Vec<_> = ["read-invalid", "write-invalid", "execute-invalid", "instruction-pointer", "other"]
            .into_iter()
            .enumerate()
            .filter_map(|(i, kind)| ((self.address_kinds & (1 << i)) != 0).then_some(kind))
            .collect();
        format!(
            "{status}; offered {}; enabled {}; {} addresses, {} vendor records{}{}",
            self.offered.fault,
            self.fault_enabled,
            self.addresses,
            self.vendors,
            if kinds.is_empty() {
                String::new()
            } else {
                format!("; {}", kinds.join(", "))
            },
            if self.vendors > 0 {
                "; vendor detail available locally"
            } else {
                ""
            }
        )
    }
}

impl Report {
    /// Numeric on-disk schema. The reader reconstructs public text from the
    /// closed dictionary; a tampered artifact cannot publish arbitrary text.
    pub fn encode_public(&self) -> String {
        let caps = u8::from(self.offered.amd)
            | (u8::from(self.offered.nv) << 1)
            | (u8::from(self.offered.fault) << 2)
            | (u8::from(self.offered.coherent) << 3);
        format!(
            "v1 {caps} {} {} {} {} {} {} {} {} {} {} {}\n",
            self.method as u8,
            u8::from(self.fault_enabled),
            self.generation,
            self.observed.map_or(0, |o| o.frame),
            self.observed.map_or(0, |o| o.boundary as u8 + 1),
            self.stage as u8,
            u8::from(self.saturated),
            self.fault as u8,
            self.addresses,
            self.vendors,
            self.address_kinds
        )
    }
    pub fn decode_public(text: &str) -> Option<Self> {
        const METHODS: [Method; 4] = [Method::Unavailable, Method::Amd, Method::Nv, Method::Transfer];
        const STAGES: [Stage; 5] = [Stage::Unavailable, Stage::Top, Stage::Bottom, Stage::Other, Stage::Transfer];
        const FAULTS: [FaultStatus; 10] = [
            FaultStatus::Unsupported,
            FaultStatus::Disabled,
            FaultStatus::Pending,
            FaultStatus::NoDetail,
            FaultStatus::Failed,
            FaultStatus::Truncated,
            FaultStatus::Collected,
            FaultStatus::Simulated,
            FaultStatus::Unconfirmed,
            FaultStatus::NotRequested,
        ];
        const BOUNDARIES: [Boundary; 22] = [
            Boundary::FrameBegin,
            Boundary::ShadowsBegin,
            Boundary::ShadowsEnd,
            Boundary::DepthBegin,
            Boundary::DepthEnd,
            Boundary::GtaoBegin,
            Boundary::GtaoEnd,
            Boundary::ContactBegin,
            Boundary::ContactEnd,
            Boundary::ReflectionsBegin,
            Boundary::ReflectionsEnd,
            Boundary::ProbesBegin,
            Boundary::ProbesEnd,
            Boundary::SceneBegin,
            Boundary::SceneEnd,
            Boundary::SsrWaterBlendBegin,
            Boundary::SsrWaterBlendEnd,
            Boundary::PostBegin,
            Boundary::PostEnd,
            Boundary::FrameEnd,
            Boundary::PhotoBegin,
            Boundary::PhotoEnd,
        ];
        if text.len() > 512 {
            return None;
        }
        let mut fields = text.split_whitespace();
        if fields.next()? != "v1" {
            return None;
        }
        let numbers: Vec<u64> = fields.map(str::parse).collect::<Result<_, _>>().ok()?;
        let [
            caps,
            method,
            enabled,
            generation,
            frame,
            boundary,
            stage,
            saturated,
            fault,
            addresses,
            vendors,
            kinds,
        ] = numbers.as_slice()
        else {
            return None;
        };
        if *caps > 15 || *enabled > 1 || *saturated > 1 || *addresses > 1024 || *vendors > 256 || *kinds > 31 {
            return None;
        }
        let observed = if *boundary == 0 {
            None
        } else {
            Some(Observed {
                frame: *frame,
                boundary: *BOUNDARIES.get(usize::try_from(*boundary - 1).ok()?)?,
                #[cfg(any(not(target_os = "macos"), test))]
                token: 0,
            })
        };
        Some(Self {
            offered: Capabilities {
                amd: caps & 1 != 0,
                nv: caps & 2 != 0,
                fault: caps & 4 != 0,
                coherent: caps & 8 != 0,
            },
            method: *METHODS.get(usize::try_from(*method).ok()?)?,
            fault_enabled: *enabled == 1,
            generation: *generation,
            observed,
            stage: *STAGES.get(usize::try_from(*stage).ok()?)?,
            saturated: *saturated == 1,
            fault: *FAULTS.get(usize::try_from(*fault).ok()?)?,
            addresses: *addresses as u32,
            vendors: *vendors as u32,
            address_kinds: *kinds as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "macos"))]
    use std::sync::Arc;
    use std::sync::{Mutex, OnceLock};
    #[test]
    fn breadcrumb_xr_eyes_with_one_frame_number_keep_independent_lifetimes() {
        let mut ring = Ring::new(1);
        let left = ring.reserve(100).unwrap();
        let right = ring.reserve(100).unwrap();
        assert_ne!(left, right);
        ring.complete(left);
        assert!(ring.current(right).is_some());
        let replacement = ring.reserve(101).unwrap();
        assert_ne!(replacement.slot, right.slot);
    }
    #[test]
    fn breadcrumb_public_report_round_trips_but_rejects_private_text_and_unbounded_counts() {
        let report = Report {
            offered: Capabilities {
                amd: true,
                fault: true,
                ..Capabilities::default()
            },
            method: Method::Amd,
            generation: 3,
            observed: Some(Observed {
                frame: 100,
                boundary: Boundary::ContactBegin,
                token: 19,
            }),
            stage: Stage::Top,
            fault: FaultStatus::Collected,
            fault_enabled: true,
            addresses: 2,
            vendors: 1,
            address_kinds: 2,
            ..Report::default()
        };
        let decoded = Report::decode_public(&report.encode_public()).unwrap();
        assert_eq!(decoded.breadcrumbs(), report.breadcrumbs());
        assert_eq!(decoded.fault(), report.fault());
        assert!(decoded.fault().contains("write-invalid"));
        for malicious in [
            "private-driver-path",
            "v1 0 0 0 1 0 0 0 0 6 4294967295 0 0",
            "v1 0 0 0 1 0 99 0 0 6 0 0 0",
        ] {
            assert!(Report::decode_public(malicious).is_none());
        }
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn breadcrumb_stuck_driver_worker_never_blocks_report_or_starts_another_worker() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let report = Arc::new(Mutex::new(Report {
            fault: FaultStatus::Pending,
            ..Report::default()
        }));
        let (started, ready) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let target = report.clone();
        let worker = crate::gpu_diagnostics::recorder::spawn_jobs("test-gpu-fault", move |_: ()| {
            started.send(()).unwrap();
            held.recv().unwrap(); // Models a driver call with no deadline.
            target.lock().unwrap().fault = FaultStatus::NoDetail;
        })
        .unwrap();
        worker.try_send(()).unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let begin = Instant::now();
        assert!(report.try_lock().unwrap().fault().contains("not collected within budget"));
        assert!(begin.elapsed() < Duration::from_millis(100));
        // Test cleanup releases the fake call; production never waits/joins.
        release.send(()).unwrap();
    }
    #[test]
    fn breadcrumb_recovery_does_not_replace_first_loss_snapshot() {
        let first = OnceLock::new();
        first
            .set(Mutex::new(Report {
                generation: 7,
                ..Report::default()
            }))
            .unwrap();
        assert!(
            first
                .set(Mutex::new(Report {
                    generation: 8,
                    ..Report::default()
                }))
                .is_err()
        );
        assert_eq!(first.get().unwrap().lock().unwrap().generation, 7);
    }
    #[test]
    fn breadcrumb_ring_never_reuses_inflight_slots_and_rejects_old_epochs() {
        let mut ring = Ring::new(7);
        let first = ring.reserve(4).unwrap();
        let (offset, token) = ring.record(first, Boundary::ContactBegin).unwrap();
        let other: Vec<_> = (1..SLOTS).map(|n| ring.reserve(n as u64).unwrap()).collect();
        assert!(ring.reserve(65).is_none());
        assert!(ring.saturated);
        assert_eq!(ring.decode(7, offset, token).unwrap().boundary, Boundary::ContactBegin);
        assert!(ring.decode(8, offset, token).is_none());
        ring.complete(first);
        let replacement = ring.reserve(66).unwrap();
        let (new_offset, new_token) = ring.record(replacement, Boundary::PostEnd).unwrap();
        assert_eq!(new_offset, offset);
        assert!(ring.decode(7, offset, token).is_none());
        assert_eq!(ring.checkpoint(new_token).unwrap().frame, 66);
        ring.complete(first); // stale completion cannot free the new frame.
        assert!(ring.reserve(67).is_none());
        for ticket in other {
            ring.complete(ticket);
        }
        assert!(ring.reserve(68).is_some());
    }
    #[test]
    fn breadcrumb_tokens_stop_before_wrap_and_metadata_stays_bounded() {
        let mut ring = Ring::new(1);
        ring.epoch = LAST_EPOCH;
        assert!(ring.reserve(1).is_none());
        assert!(std::mem::size_of::<Ring>() <= 64 * 1024);
        assert_eq!(PAYLOAD_WORDS * 4, 16 * 1024);
    }
    #[test]
    fn breadcrumb_words_are_independent_and_overflow_is_ignored() {
        let mut ring = Ring::new(1);
        let ticket = ring.reserve(10).unwrap();
        for n in 0..WORDS {
            let (offset, token) = ring.record(ticket, Boundary::DepthEnd).unwrap();
            assert_eq!(offset, n);
            assert_eq!(ring.decode(1, offset, token).unwrap().frame, 10);
            assert!(ring.decode(1, offset, token.wrapping_add(1)).is_none());
        }
        assert!(ring.record(ticket, Boundary::PostBegin).is_none());
    }
    #[test]
    fn breadcrumb_capabilities_use_runtime_offers_and_expensive_fallback_is_opt_in() {
        assert!(Boundary::ContactBegin.begin());
        assert!(!Boundary::ContactEnd.begin());
        assert_eq!(Capabilities::default().select(Mode::Auto), Method::Unavailable);
        assert_eq!(Capabilities::default().select(Mode::Transfer), Method::Transfer);
        let amd = Capabilities {
            amd: true,
            ..Capabilities::default()
        };
        assert_eq!(amd.select(Mode::Off), Method::Unavailable);
        assert_eq!(amd.select(Mode::Auto), Method::Amd);
        assert_eq!(
            Capabilities {
                nv: true,
                ..Capabilities::default()
            }
            .select(Mode::Auto),
            Method::Nv
        );
    }
    #[test]
    fn breadcrumb_loss_before_first_submission_is_unavailable_not_a_cause() {
        let report = Report::default();
        assert!(report.breadcrumbs().contains("last observed boundary unavailable"));
        assert!(!report.breadcrumbs().contains("caused"));
        assert!(report.fault().starts_with("unsupported"));
    }
}
