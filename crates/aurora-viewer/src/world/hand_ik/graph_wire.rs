//! Atomic bounded multipart definitions. Only the already authenticated source
//! session reaches this parser; partial or invalid snapshots never renew a lease.
use super::*;
use crate::world::rig_graph::{Definition, MAX_BYTES};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::Arc;

pub const PREFIX: &str = "auroraIK/5 ";

pub fn observer_subject(message: &str) -> Option<Uuid> {
    if message.len() > 1024 {
        return None;
    }
    let packet: Packet = serde_json::from_str(message.strip_prefix(PREFIX)?).ok()?;
    packet.avatar.filter(|id| !id.is_nil())
}
const MAX_PARTS: usize = 48;

/// Stable across heartbeats, fresh across seat sessions and script restarts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationStamp {
    pub session: Uuid,
    pub epoch: Uuid,
    pub bundle: Uuid,
    pub cycles: [u32; 16],
}

#[derive(Clone)]
pub struct Runtime {
    pub source: Seat,
    pub definition: Arc<Definition>,
    pub anchor: Instant,
    pub phase: f64,
    pub bones: Vec<String>,
    pub bindings: Vec<usize>,
    pub smoothing: Option<[f32; 2]>,
    pub activation: ActivationStamp,
}

impl Runtime {
    pub fn time(&self, now: Instant) -> f64 {
        self.phase + now.saturating_duration_since(self.anchor).as_secs_f64()
    }
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Packet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    avatar: Option<Uuid>,
    nonce: Uuid,
    revision: u32,
    bundle: Uuid,
    epoch: Uuid,
    enabled: bool,
    #[serde(default)]
    mask: u32,
    #[serde(default)]
    profile: String,
    #[serde(default)]
    cycles: Vec<u32>,
    time: f64,
    part: Option<usize>,
    count: Option<usize>,
    data: Option<String>,
}
struct Pending {
    header: Packet,
    parts: Vec<Option<Vec<u8>>>,
    at: Instant,
}
#[derive(Default)]
pub(super) struct State {
    session: Uuid,
    nonce: Uuid,
    revision: Option<u32>,
    bundle: Option<Uuid>,
    pub definition: Option<Arc<Definition>>,
    pub enabled: bool,
    pub disabled: Vec<String>,
    pub clock: Option<(Instant, f64)>,
    pending: Option<Pending>,
    snapshot_serial: u64,
    epoch: Uuid,
    mask: u32,
    cycles: [u32; 16],
    selection: profiles::Selection,
    definition_bytes: usize,
    pub(super) diagnostics: super::diagnostics::WireCounters,
}
impl State {
    fn reject(&mut self) -> bool {
        if !matches!(
            self.diagnostics.last,
            "awaiting_parts" | "stale_revision" | "state_without_definition" | "older_partial_revision"
        ) {
            self.diagnostics.failure = self.diagnostics.last;
        }
        false
    }

    pub(super) fn describe(&self, out: &mut String) {
        use std::fmt::Write as _;
        let d = &self.diagnostics;
        let _ = writeln!(
            out,
            "  packets={} applied={} active_packets={} max_bytes={} last={} last_failure={}",
            d.received, d.applied, d.active, d.max_bytes, d.last, d.failure
        );
        let _ = writeln!(
            out,
            "  definition={} enabled={} profile={:?} bytes={} snapshots={}",
            self.definition.is_some(),
            self.enabled,
            super::diagnostics::short(&self.selection.name),
            self.definition_bytes,
            self.snapshot_serial
        );
        if let Some(pending) = &self.pending {
            let received = pending.parts.iter().filter(|p| p.is_some()).count();
            let _ = writeln!(out, "  pending_parts={received}/{}", pending.parts.len());
        }
    }

    pub fn definition_bytes(&self) -> usize {
        self.definition_bytes
    }
    pub fn retained_bytes(&self) -> usize {
        self.definition_bytes
            + self
                .pending
                .as_ref()
                .map_or(0, |p| p.parts.iter().flatten().map(Vec::len).sum::<usize>())
    }
    pub fn expired_transfer(&self, now: Instant) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| now.saturating_duration_since(p.at) >= Duration::from_secs(15))
    }
    pub fn prune(&mut self, now: Instant) {
        if self.expired_transfer(now) {
            self.pending = None;
        }
    }
    pub fn profile_allows(&self, index: usize, bone: &str) -> bool {
        self.definition.as_ref().is_none_or(|d| self.selection.allows(d, index, bone))
    }
    pub fn smoothing(&self) -> Option<[f32; 2]> {
        self.selection.smoothing(self.definition.as_ref()?)
    }
    pub fn binding_enabled(&self, index: usize) -> bool {
        index < 16 && self.mask & (1 << index) == 0
    }
    pub fn activation(&self) -> ActivationStamp {
        let mut cycles = self.cycles;
        if let Some(definition) = &self.definition {
            for (index, cycle) in cycles.iter_mut().enumerate().take(definition.bindings.len()) {
                *cycle = cycle.wrapping_add(self.selection.cycle(index));
            }
        }
        ActivationStamp {
            session: self.session,
            epoch: self.epoch,
            bundle: self.bundle.unwrap_or_default(),
            cycles,
        }
    }
    pub fn snapshot_serial(&self) -> u64 {
        self.snapshot_serial
    }
    pub fn cancel_transfer(&mut self) {
        self.pending = None;
    }
    pub fn transferring(&self, now: Instant) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| now.saturating_duration_since(p.at) < Duration::from_secs(15))
    }
    pub fn bundle(&self) -> Option<Uuid> {
        self.bundle
    }
    #[cfg(test)]
    pub fn receive(&mut self, message: &str, avatar: Uuid, nonce: Uuid, now: Instant) -> bool {
        self.receive_bounded(message, avatar, nonce, now, 2 * MAX_BYTES)
    }
    pub fn receive_bounded(&mut self, message: &str, avatar: Uuid, nonce: Uuid, now: Instant, allowance: usize) -> bool {
        self.diagnostics.received = self.diagnostics.received.saturating_add(1);
        self.diagnostics.max_bytes = self.diagnostics.max_bytes.max(message.len());
        self.diagnostics.last = "envelope_or_missing_request";
        if message.len() > 1024 || nonce.is_nil() {
            return self.reject();
        }
        let Some(body) = message.strip_prefix(PREFIX) else {
            return self.reject();
        };
        self.diagnostics.last = "invalid_packet_json";
        let Ok(p) = serde_json::from_str::<Packet>(body) else {
            return self.reject();
        };
        self.diagnostics.last = "identity_or_header_bounds";
        if p.avatar != Some(avatar)
            || p.nonce != nonce
            || p.bundle.is_nil()
            || p.epoch.is_nil()
            || !p.time.is_finite()
            || !(0.0..=1_000_000_000.0).contains(&p.time)
            || p.mask > 0xffff
            || p.profile.len() > 64
            || p.cycles.len() > 16
            || p.cycles.iter().any(|c| *c > i32::MAX as u32)
        {
            return self.reject();
        }
        if self.nonce != nonce {
            self.nonce = nonce;
            self.revision = None;
            self.pending = None;
        }
        self.diagnostics.last = "stale_revision";
        if self.revision.is_some_and(|r| p.revision <= r) {
            return self.reject();
        }
        let finish = |s: &mut Self, p: &Packet, at: Instant| {
            // Request nonces rotate on every renewal and authenticate replies.
            // A source-use lifetime changes only when its State is discarded;
            // genuine script/contact restarts still use epoch/bundle/cycles.
            if s.session.is_nil() {
                s.session = Uuid::new_v4();
            }
            for (index, cycle) in s.cycles.iter_mut().enumerate() {
                if s.mask & (1 << index) != 0 && p.mask & (1 << index) == 0 {
                    *cycle = cycle.wrapping_add(1);
                }
            }
            s.mask = p.mask;
            s.epoch = p.epoch;
            s.bundle = Some(p.bundle);
            s.revision = Some(p.revision);
            s.enabled = p.enabled;
            s.selection = profiles::Selection {
                name: p.profile.clone(),
                cycles: p.cycles.clone(),
            };
            s.disabled = s.definition.as_ref().map_or_else(Vec::new, |d| {
                d.bindings
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| p.mask & (1 << i) != 0)
                    .map(|(_, b)| b.bone.clone())
                    .collect()
            });
            s.clock = Some((at, p.time));
            s.pending = None;
            s.diagnostics.applied = s.diagnostics.applied.saturating_add(1);
            s.diagnostics.active = s.diagnostics.active.saturating_add(u64::from(p.enabled));
            s.diagnostics.last = "applied_state";
        };
        if p.part.is_none() && p.count.is_none() && p.data.is_none() {
            if !p.enabled && self.bundle != Some(p.bundle) {
                self.enabled = false;
                self.revision = Some(p.revision);
                self.pending = None;
                self.diagnostics.last = "off_without_definition";
                return true;
            }
            self.diagnostics.last = "state_without_definition";
            if self.bundle != Some(p.bundle) || self.definition.is_none() {
                return self.reject();
            }
            self.diagnostics.last = "invalid_state_profile";
            if !self.definition.as_ref().is_some_and(|d| {
                profiles::Selection {
                    name: p.profile.clone(),
                    cycles: p.cycles.clone(),
                }
                .valid(d, p.enabled)
            }) {
                return self.reject();
            }
            finish(self, &p, now);
            return true;
        }
        self.diagnostics.last = "fragment_bounds";
        let (Some(part), Some(count), Some(data)) = (p.part, p.count, p.data.as_ref()) else {
            return self.reject();
        };
        if count == 0 || count > MAX_PARTS || part >= count || data.len() > 700 {
            return self.reject();
        }
        self.diagnostics.last = "invalid_base64";
        let Ok(bytes) = STANDARD.decode(data) else { return self.reject() };
        self.diagnostics.last = "decoded_fragment_bounds";
        if bytes.len() > 512 {
            return self.reject();
        }
        self.prune(now);
        self.diagnostics.last = "older_partial_revision";
        if self.pending.as_ref().is_some_and(|s| s.header.revision > p.revision) {
            return self.reject();
        }
        if self.pending.as_ref().is_none_or(|s| s.header.revision != p.revision) {
            self.pending = Some(Pending {
                header: p.clone(),
                parts: vec![None; count],
                at: now,
            });
        }
        self.diagnostics.last = "inconsistent_fragment_header";
        let Some(pending) = self.pending.as_mut() else {
            return self.reject();
        };
        let h = &pending.header;
        if h.bundle != p.bundle
            || h.epoch != p.epoch
            || h.enabled != p.enabled
            || h.mask != p.mask
            || h.profile != p.profile
            || h.cycles != p.cycles
            || h.time != p.time
            || pending.parts.len() != count
        {
            return self.reject();
        }
        self.diagnostics.last = "conflicting_fragment";
        if pending.parts[part].as_ref().is_some_and(|old| *old != bytes) {
            self.pending = None;
            return self.reject();
        }
        let total = pending
            .parts
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != part)
            .filter_map(|(_, b)| b.as_ref())
            .map(Vec::len)
            .sum::<usize>()
            + bytes.len();
        self.diagnostics.last = "definition_byte_budget";
        if total > MAX_BYTES || self.definition_bytes + total > allowance {
            self.pending = None;
            return self.reject();
        }
        pending.parts[part] = Some(bytes);
        self.diagnostics.last = "awaiting_parts";
        if pending.parts.iter().any(Option::is_none) {
            return self.reject();
        }
        let bytes: Vec<u8> = pending.parts.iter().flatten().flatten().copied().collect();
        self.diagnostics.last = "invalid_definition_json";
        let Ok(definition) = serde_json::from_slice::<Definition>(&bytes) else {
            self.pending = None;
            return self.reject();
        };
        self.diagnostics.last = "invalid_definition_or_profile";
        if definition.validate().is_err()
            || !(profiles::Selection {
                name: p.profile.clone(),
                cycles: p.cycles.clone(),
            })
            .valid(&definition, p.enabled)
        {
            self.pending = None;
            return self.reject();
        }
        let at = pending.at;
        self.definition_bytes = total;
        self.definition = Some(Arc::new(definition));
        self.snapshot_serial = self.snapshot_serial.wrapping_add(1);
        finish(self, &p, at);
        self.diagnostics.last = "complete_definition";
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_bone_slots_keep_independent_wire_masks_and_activation_generations() {
        let avatar = Uuid::new_v4();
        let nonce = Uuid::new_v4();
        let bundle = Uuid::new_v4();
        let now = Instant::now();
        let definition = serde_json::json!({"bindings":[
            {"bone":"mHead","target":{"kind":"source"}},
            {"bone":"mHead","target":{"kind":"source"},"contact":[0.01,0,0]}
        ]});
        let mut body = serde_json::json!({"avatar":avatar,"nonce":nonce,"bundle":bundle,"epoch":bundle,
            "revision":1,"enabled":true,"time":0.0,"mask":1,"cycles":[1,1],"part":0,"count":1,"data":STANDARD.encode(definition.to_string())});
        let mut state = State::default();
        assert!(state.receive(&format!("{PREFIX}{body}"), avatar, nonce, now));
        assert!(!state.binding_enabled(0) && state.binding_enabled(1));
        assert_eq!(state.definition.as_ref().unwrap().bindings.len(), 2);
        let previous = state.activation();
        for field in ["part", "count", "data"] {
            body.as_object_mut().unwrap().remove(field);
        }
        body["revision"] = 2.into();
        body["mask"] = 2.into();
        body["cycles"] = serde_json::json!([2, 1]);
        assert!(state.receive(&format!("{PREFIX}{body}"), avatar, nonce, now));
        assert!(state.binding_enabled(0) && !state.binding_enabled(1));
        let current = state.activation();
        assert_ne!(previous.cycles[0], current.cycles[0]);
        assert_eq!(previous.cycles[1], current.cycles[1]);
    }
    #[test]
    fn named_profiles_validate_atomic_generations_and_preserve_lifetimes_across_equivalent_presets() {
        let avatar = Uuid::new_v4();
        let nonce = Uuid::new_v4();
        let bundle = Uuid::new_v4();
        let now = Instant::now();
        let definition = serde_json::json!({
            "bindings":[{"bone":"mHead","target":{"kind":"source"}}],
            "profiles":[{"name":"drive","bones":["mHead"]},{"name":"gas_only","bones":["mHead"]}]
        });
        let header = |revision, profile, cycles: serde_json::Value| {
            serde_json::json!({
                "avatar":avatar,"nonce":nonce,"bundle":bundle,"epoch":bundle,
                "revision":revision,"profile":profile,"cycles":cycles,"enabled":true,"time":0.0
            })
        };
        let message = |body: &serde_json::Value| format!("{PREFIX}{body}");
        let mut full = header(1, "drive", serde_json::json!([1]));
        full["part"] = 0.into();
        full["count"] = 1.into();
        full["data"] = STANDARD.encode(definition.to_string()).into();
        let mut state = State::default();
        assert!(state.receive(&message(&full), avatar, nonce, now));
        let first = state.activation();
        assert!(state.receive(&message(&header(2, "gas_only", serde_json::json!([1]))), avatar, nonce, now));
        assert_eq!(
            state.activation(),
            first,
            "the same binding keeps its clock across a profile switch"
        );
        for (profile, cycles) in [
            ("missing", serde_json::json!([1])),
            ("drive", serde_json::json!([])),
            ("drive", serde_json::json!([1, 2])),
            ("drive", serde_json::json!([-1])),
            ("drive", serde_json::json!([2147483648_u32])),
        ] {
            assert!(!state.receive(&message(&header(3, profile, cycles)), avatar, nonce, now));
            assert_eq!(state.activation(), first);
        }
        assert!(state.receive(&message(&header(4, "drive", serde_json::json!([2]))), avatar, nonce, now));
        assert_ne!(
            state.activation(),
            first,
            "a missed release is detected from the new binding generation"
        );
        let parts: Vec<_> = definition.to_string().as_bytes().chunks(64).map(|b| STANDARD.encode(b)).collect();
        let part = |index: usize, cycle| {
            let mut body = header(5, "drive", serde_json::json!([cycle]));
            body["part"] = serde_json::json!(index);
            body["count"] = parts.len().into();
            body["data"] = parts[index].clone().into();
            message(&body)
        };
        assert!(!state.receive(&part(0, 3), avatar, nonce, now));
        assert!(!state.receive(&part(1, 4), avatar, nonce, now), "a transfer cannot mix generations");
        assert_eq!(state.revision, Some(4));
        for index in 1..parts.len() {
            state.receive(&part(index, 3), avatar, nonce, now);
        }
        assert_eq!(state.revision, Some(5));
    }
    #[test]
    fn activation_epochs_reset_lifetimes_without_invalidating_cached_definitions_and_masks_restart_only_their_bone() {
        let avatar = Uuid::new_v4();
        let nonce = Uuid::new_v4();
        let bundle = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let now = Instant::now();
        let rig = serde_json::json!({"bindings":[
            {"bone":"mHead","target":{"kind":"bone","bone":"mPelvis"}},
            {"bone":"mWristLeft","target":{"kind":"bone","bone":"mPelvis"}}]});
        let header = |revision, epoch, enabled, mask| {
            serde_json::json!({
                "avatar":avatar,"nonce":nonce,"bundle":bundle,"epoch":epoch,
                "revision":revision,"enabled":enabled,"mask":mask,"time":0.0
            })
        };
        let message = |body: &serde_json::Value| format!("{PREFIX}{body}");
        let mut full = header(1, first, true, 0);
        full["part"] = 0.into();
        full["count"] = 1.into();
        full["data"] = STANDARD.encode(rig.to_string()).into();
        let mut state = State::default();
        assert!(
            !state.receive(&format!("auroraIK/4 {full}"), avatar, nonce, now),
            "only the current activation-aware protocol is accepted"
        );
        assert!(state.receive(&message(&full), avatar, nonce, now));
        let cached = state.definition.clone().unwrap();
        let original = state.activation();
        assert!(state.receive(&message(&header(2, first, false, 0)), avatar, nonce, now));
        assert!(state.receive(&message(&header(3, second, true, 0)), avatar, nonce, now));
        assert_ne!(state.activation(), original);
        assert_eq!(state.activation().epoch, second);
        assert!(Arc::ptr_eq(&cached, state.definition.as_ref().unwrap()));
        assert!(state.receive(&message(&header(4, second, true, 1)), avatar, nonce, now));
        assert!(state.receive(&message(&header(5, second, true, 0)), avatar, nonce, now));
        assert_eq!(state.activation().cycles[0], 1);
        assert_eq!(state.activation().cycles[1], 0);
        let stable = state.activation();
        for epoch in [serde_json::Value::Null, serde_json::json!(Uuid::nil())] {
            let mut invalid = header(6, second, true, 0);
            invalid["epoch"] = epoch;
            assert!(!state.receive(&message(&invalid), avatar, nonce, now));
        }
        let mut missing = header(6, second, true, 0);
        missing.as_object_mut().unwrap().remove("epoch");
        assert!(!state.receive(&message(&missing), avatar, nonce, now));
        assert_eq!(state.activation(), stable);
        let mut maximum = header(u32::MAX, second, true, 0xffff);
        maximum["time"] = serde_json::json!(1_000_000_000.0);
        maximum["profile"] = "x".repeat(64).into();
        maximum["cycles"] = serde_json::json!(vec![i32::MAX; 16]);
        maximum["part"] = 47.into();
        maximum["count"] = 48.into();
        maximum["data"] = STANDARD.encode([0; 320]).into();
        assert!(
            message(&maximum).len() <= 1024,
            "the activation epoch must fit the simulator chat limit"
        );
    }
    #[test]
    fn rig_packets_commit_atomically_out_of_order_cache_heartbeats_and_reject_stale_or_conflicting_data() {
        let avatar = Uuid::new_v4();
        let nonce = Uuid::new_v4();
        let bundle = Uuid::new_v4();
        let now = Instant::now();
        let rig = serde_json::json!({"nodes":[],"bindings":[{"bone":"mHead","target":{"kind":"bone","bone":"mPelvis"}}]});
        let bytes = serde_json::to_vec(&rig).unwrap();
        let parts: Vec<_> = bytes.chunks(48).map(|b| STANDARD.encode(b)).collect();
        let packet = |revision, part: usize| {
            format!(
                "{PREFIX}{}",
                serde_json::json!({"avatar":avatar,"nonce":nonce,"revision":revision,"bundle":bundle,"epoch":bundle,"enabled":true,"mask":0,"time":1.0,"part":part,"count":parts.len(),"data":parts[part]})
            )
        };
        let mut state = State::default();
        for part in (1..parts.len()).rev() {
            assert!(!state.receive(&packet(1, part), avatar, nonce, now));
            assert!(state.definition.is_none());
        }
        assert!(state.receive(&packet(1, 0), avatar, nonce, now));
        assert_eq!(state.definition.as_ref().unwrap().bindings.len(), 1);
        let heartbeat = |revision, enabled, mask| {
            format!(
                "{PREFIX}{}",
                serde_json::json!({"avatar":avatar,"nonce":nonce,"revision":revision,"bundle":bundle,"epoch":bundle,"enabled":enabled,"mask":mask,"time":2.0})
            )
        };
        assert!(state.receive(&heartbeat(2, true, 1), avatar, nonce, now));
        assert_eq!(state.disabled, vec!["mHead"]);
        assert!(!state.receive(&packet(1, 0), avatar, nonce, now));
        assert!(!state.receive(&heartbeat(3, true, 0), avatar, Uuid::new_v4(), now));
        assert!(!state.receive(&packet(3, 0), avatar, nonce, now));
        let mut conflict: serde_json::Value = serde_json::from_str(packet(3, 0).strip_prefix(PREFIX).unwrap()).unwrap();
        conflict["data"] = serde_json::json!(STANDARD.encode(b"different"));
        assert!(!state.receive(&format!("{PREFIX}{conflict}"), avatar, nonce, now));
        assert!(state.definition.is_some());
        assert_eq!(state.revision, Some(2));
        let mut long: serde_json::Value = serde_json::from_str(heartbeat(3, true, 0).strip_prefix(PREFIX).unwrap()).unwrap();
        long["time"] = serde_json::json!(100_000_000.0);
        assert!(state.receive(&format!("{PREFIX}{long}"), avatar, nonce, now));
        assert_eq!(state.clock, Some((now, 100_000_000.0)));
        assert!(state.receive(&heartbeat(4, false, 0), avatar, nonce, now));
        assert!(!state.enabled);
    }
}
