//! On-demand bounded IK diagnostics. Never retain packets, authentication IDs
//! or resident identities, and never issue a query while preparing a report.
use super::*;
use crate::world::rig_graph::Target;
use std::fmt::Write as _;

#[derive(Default)]
pub(super) struct WireCounters {
    pub received: u64,
    pub applied: u64,
    pub active: u64,
    pub max_bytes: usize,
    pub last: &'static str,
    pub failure: &'static str,
}

#[derive(Default)]
pub(super) struct RelayCounters {
    pub envelopes: u64,
    pub forwarded: u64,
    pub max_bytes: usize,
    pub last: &'static str,
    pub failure: &'static str,
}

impl World {
    pub(crate) fn describe_ik(&self, editing: bool, suppressed: bool, edit_goals: usize, now: Instant) -> String {
        let contacts = &self.hand_contacts;
        let mut out = String::from("IK diagnostic: local avatar\n");
        let relay = &contacts.relay_diagnostics;
        let _ = writeln!(
            out,
            "relay_ready={} envelopes={} forwarded={} max_bytes={} last={} last_failure={}",
            contacts.relay.is_some(),
            relay.envelopes,
            relay.forwarded,
            relay.max_bytes,
            relay.last,
            relay.failure
        );
        let _ = writeln!(out, "editor_open={editing} suppress_scripts={suppressed} editor_goals={edit_goals}");
        let _ = writeln!(
            out,
            "announcements_bytes={} local_sources={} observer_avatars={} active_graphs={}",
            contacts.declarations.bytes(),
            contacts.sources().len(),
            contacts.observers.len(),
            contacts.graphs().len()
        );
        let mut sessions: Vec<_> = contacts
            .sessions()
            .chain(std::iter::once(&contacts.editor))
            .filter(|s| s.seat.is_some())
            .collect();
        sessions.sort_by_key(|s| s.seat.map(|seat| seat.root));
        for (number, session) in sessions.iter().take(8).enumerate() {
            let Some(source) = session.seat else { continue };
            let editor = std::ptr::eq(*session, &contacts.editor);
            let _ = writeln!(
                out,
                "Source {}: editor={} eligible={} probes={} complete_ack={} advertised_matches={} last_request_ago_s={}",
                number + 1,
                editor,
                sources::eligible_for(self, self.agent_id, source),
                session.probes,
                session.received_at.is_some(),
                session.advertised_bundle.is_some_and(|b| session.graph.bundle() == Some(b)),
                session.sent_at.map_or_else(
                    || "none".into(),
                    |at| format!("{:.2}", now.saturating_duration_since(at).as_secs_f64())
                )
            );
            session.graph.describe(&mut out);
            if let Some(definition) = &session.graph.definition {
                let _ = writeln!(
                    out,
                    "  bindings={} virtual_nodes={}",
                    definition.bindings.len(),
                    definition.nodes.len()
                );
                for (index, binding) in definition.bindings.iter().take(16).enumerate() {
                    let selected = session.graph.profile_allows(index, &binding.bone) && session.graph.binding_enabled(index);
                    let target = match &binding.target {
                        Target::Link { object, link } => format!(
                            "link {:?}: {}",
                            short(link),
                            self.ik_targets.describe_link(self, source, *object, link)
                        ),
                        Target::Prim { id, .. } => format!("prim: present={}", self.objects.index_of_uuid(id).is_some()),
                        Target::Bone { avatar, bone } => format!(
                            "{} bone {:?}",
                            if avatar.is_nil() || *avatar == self.agent_id {
                                "own"
                            } else {
                                "other"
                            },
                            short(bone)
                        ),
                        Target::Node { .. } => "virtual node".into(),
                        Target::Source => "source root".into(),
                    };
                    let _ = writeln!(
                        out,
                        "  #{} {} selected={} enabled={} weight={:.3} distance={:.3} target={}",
                        index, binding.bone, selected, binding.enabled, binding.weight, binding.activation.distance, target
                    );
                }
            }
        }
        if sessions.len() > 8 {
            out.push_str("More sources omitted (report limit).\n");
        }
        out
    }
}

pub(super) fn short(value: &str) -> String {
    value.chars().take(64).map(|c| if c.is_control() { '?' } else { c }).collect()
}
