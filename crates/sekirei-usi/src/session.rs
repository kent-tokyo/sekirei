//! Lifecycle helpers for the single in-flight USI search generation.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

/// Abort and join an in-flight search before shared search state is mutated.
pub(crate) fn abort_and_join(
    search_abort: &mut Option<Arc<AtomicBool>>,
    search_handle: &mut Option<JoinHandle<()>>,
) {
    if let Some(abort) = search_abort.take() {
        abort.store(true, Ordering::Relaxed);
    }
    if let Some(handle) = search_handle.take() {
        handle.join().ok();
    }
}

/// Supersede the current USI question and prevent a late result from crossing
/// the command boundary before joining its worker.
pub(crate) fn invalidate_and_join(
    search_generation: &AtomicU64,
    search_abort: &mut Option<Arc<AtomicBool>>,
    search_handle: &mut Option<JoinHandle<()>>,
) {
    search_generation.fetch_add(1, Ordering::AcqRel);
    abort_and_join(search_abort, search_handle);
}

/// Change process-wide evaluator state only after the active search can no
/// longer observe it.
pub(crate) fn mutate_after_join<T>(
    search_abort: &mut Option<Arc<AtomicBool>>,
    search_handle: &mut Option<JoinHandle<()>>,
    mutate: impl FnOnce() -> T,
) -> T {
    abort_and_join(search_abort, search_handle);
    mutate()
}
