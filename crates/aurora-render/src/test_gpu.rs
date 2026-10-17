//! One GPU device at a time across this crate's tests. The test harness
//! runs tests on parallel threads, and two tests creating and using a
//! device at once on the CI runner's software adapter ended the whole test
//! binary with an access violation (merge queue of #499, 2026-10-16). A
//! test that creates a device holds this lock until it ends.

use std::sync::{Mutex, MutexGuard};

static GPU: Mutex<()> = Mutex::new(());

/// The lock; a test that panicked while holding it does not block the
/// next ones.
pub fn lock() -> MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
