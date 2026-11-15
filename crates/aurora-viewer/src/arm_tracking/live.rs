//! Eligibility for live, render-local arm tracking. No account/session secrets
//! are inspected: login identity is compared only with the scene's own avatar.
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub avatar: Uuid,
    pub region: u64,
    pub appearance: u64,
    pub skeleton: u64,
}
