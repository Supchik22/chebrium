//! Keeping the memory of a long-running program small.
//!
//! glibc gives each thread its own heap ("arena") and keeps what a thread freed (the picture a decoder just finished
//! with, the buffers of an icon scan) for the next allocation instead of giving it back, so a small program that once
//! decoded a big image stays big. [`tame_allocator`] limits the arenas and [`trim`] hands the free pages back.

/// Call once at the start, before threads are spawned: at most two arenas, and free memory at the top of the heap goes
/// back to the system sooner.
pub fn tame_allocator() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: mallopt only sets allocator parameters
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 4 << 20);
    }
}

/// Give free memory back to the system (every arena). Cheap enough to call every few seconds.
pub fn trim() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only releases pages that are free
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Resident memory of this process in MiB (for diagnostics and tests).
pub fn resident_mib() -> f64 {
    std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<f64>().ok())).map_or(0.0, |pages| pages * 4096.0 / 1_048_576.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_gives_freed_memory_back() {
        tame_allocator();
        let before = resident_mib();
        // a big allocation made and freed on another thread, like a decoder's
        std::thread::spawn(|| {
            let mut v: Vec<Vec<u8>> = (0..64).map(|_| vec![1u8; 1 << 20]).collect();
            // touch every page, then free all but a few small pieces so the heap cannot simply shrink
            v.iter_mut().for_each(|b| b.iter_mut().step_by(4096).for_each(|x| *x = 2));
            drop(v);
        })
        .join()
        .unwrap();
        trim();
        assert!(resident_mib() < before + 24.0, "{} MiB kept after freeing 64 MiB", resident_mib() - before);
    }
}
