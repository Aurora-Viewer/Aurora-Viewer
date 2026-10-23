//! Hash maps of the paths that run every frame (scene sync, object store,
//! mesh streaming), keyed by object indices, UUIDs and geometry keys.
//!
//! The standard maps hash with SipHash, made to resist keys chosen to
//! collide: in a busy club, hashing alone was about 5 % of the main thread.
//! foldhash is several times faster on such small keys, and still seeded at
//! random per process, which is enough here: the keys are object ids the
//! simulator hands out and hashes of geometry parameters, not strings an
//! attacker picks against a known seed.

pub type FastMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;
pub type FastSet<K> = std::collections::HashSet<K, foldhash::fast::RandomState>;
