//! Runtime data belongs to live seat/wearer sessions, never a region catalog.
//! Byte budgets count serialized definitions plus incomplete replacements.
use super::*;

pub(super) const REGION_BYTES: usize = 128 * 1024;
pub(super) const AVATAR_BYTES: usize = 32 * 1024;

#[derive(Default)]
pub(super) struct Ingress {
    at: Option<Instant>,
    tokens: f64,
}
impl Ingress {
    pub fn allow(&mut self, now: Instant, rate: f64, burst: f64) -> bool {
        self.tokens = self.at.map_or(burst, |at| {
            (self.tokens + now.saturating_duration_since(at).as_secs_f64() * rate).min(burst)
        });
        self.at = Some(now);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

impl Session {
    pub(super) fn retained_bytes(&self) -> usize {
        self.graph.retained_bytes()
            + self
                .reload
                .as_ref()
                .map_or(0, |request| request.retained_bytes(self.graph.definition.as_ref()))
    }
}

impl Contacts {
    pub(super) fn sessions(&self) -> impl Iterator<Item = &Session> {
        std::iter::once(&self.primary).chain(self.worn_sessions())
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.sessions().map(Session::retained_bytes).sum::<usize>() + self.editor.retained_bytes()
    }
    pub(super) fn total_retained_bytes(&self) -> usize {
        self.retained_bytes() + self.observers.values().map(Self::retained_bytes).sum::<usize>()
    }
    pub(super) fn admits(&mut self, source: Seat, now: Instant) -> bool {
        // Ignore unknown objects before parsing their JSON or allocating data.
        let local = self.sessions().any(|s| s.seat == Some(source));
        let subject = self
            .observers
            .iter()
            .find_map(|(id, c)| c.sessions().any(|s| s.seat == Some(source)).then_some(*id));
        let session = if self.editor.seat == Some(source) && self.editor.reloading(now) {
            Some(&mut self.editor)
        } else if local {
            self.session_mut(source)
        } else {
            subject
                .and_then(|id| self.observers.get_mut(&id))
                .and_then(|c| c.session_mut(source))
        };
        session.is_some_and(|s| s.ingress.allow(now, 8.0, 64.0)) && self.ingress.allow(now, 64.0, 128.0)
    }
    pub(super) fn graph_allowance(&self, subject: Uuid, source: Seat) -> usize {
        let Some(contacts) = self.for_avatar(self.subject.unwrap_or_default(), subject) else {
            return 0;
        };
        let Some(session) = contacts.sessions().find(|s| s.seat == Some(source)) else {
            return 0;
        };
        let own = session.graph.retained_bytes();
        REGION_BYTES
            .saturating_sub(self.total_retained_bytes().saturating_sub(own))
            .min(AVATAR_BYTES.saturating_sub(contacts.retained_bytes().saturating_sub(own)))
    }
}

#[cfg(test)]
mod tests;
