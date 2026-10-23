//! Keeping glibc's heap from holding on to freed memory.
//!
//! The shell's big allocations are short-lived: a frozen screen per output
//! (~25 MB each, decoded on a worker thread), shader translation when a
//! surface is set up (a 40 MB peak), album art. glibc raises its mmap
//! threshold after the first large free, so later ones land in per-thread
//! arenas, which keep the pages after the free: measured, the installed
//! shell sat at ~140 MB anonymous where its live data is ~15 MB.

/// Above this, an allocation is a mapping of its own, returned to the
/// system when freed. Setting it also stops glibc raising it.
const MMAP_THRESHOLD: libc::c_int = 256 * 1024;
/// Arenas threads share. glibc's default is eight per core, one per
/// thread here (28 threads), each keeping its own freed pages.
const ARENA_MAX: libc::c_int = 2;

/// Set the allocator up; call before any thread starts.
pub fn tune_allocator() {
    // SAFETY: mallopt with documented parameters, before other threads.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD);
        libc::mallopt(libc::M_ARENA_MAX, ARENA_MAX);
    }
}

/// Hand freed pages back to the system, after something large closed.
pub fn release_freed() {
    // SAFETY: malloc_trim only returns free pages; it is safe at any time.
    unsafe {
        libc::malloc_trim(0);
    }
}
