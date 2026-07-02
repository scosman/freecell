//! Fresh-process peak-memory measurement for Round-2 (architecture §3).
//!
//! The authoritative peak-RSS figure for SP2 (large `.xlsx` open) is taken from a
//! **separately-spawned child process** measuring its own high-water mark, because the
//! harness process's allocator is polluted by prior work. This helper lives in the
//! frozen `round-2/harness` — NOT in `shared/bench_util`, which is frozen from Phase 1
//! and cannot gain new API — and is the single place Round-2 experiments read peak RSS.
//!
//! [`peak_rss`] returns the process **peak resident set size high-water mark, in
//! bytes**. The high-water mark is the largest RSS the process ever reached; it is
//! unaffected by later frees, which is exactly what a peak-memory metric wants.

/// Peak resident set size high-water mark of the **current process, in bytes**.
///
/// Measurement source, in priority order:
///
/// - **Linux:** `VmHWM` from `/proc/self/status` (reported in kB; converted to bytes).
///   This is the kernel's peak-RSS accounting and the most reliable figure on the
///   in-container target.
/// - **Fallback** (non-Linux, or if the `/proc` read fails / yields nothing):
///   `getrusage(RUSAGE_SELF).ru_maxrss`. **Unit caveat:** `ru_maxrss` is in **kB on
///   Linux** but in **bytes on macOS/BSD** — this function normalizes to bytes for each
///   platform so the return value is always bytes.
///
/// Returns `0` only if every source is unavailable (should not happen on the supported
/// targets).
pub fn peak_rss() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Some(bytes) = vm_hwm_bytes_linux() {
            return bytes;
        }
    }
    getrusage_maxrss_bytes()
}

/// Reads `VmHWM` (peak RSS) from `/proc/self/status`, returning bytes. `None` if the
/// file can't be read or the field is absent/unparsable.
#[cfg(target_os = "linux")]
fn vm_hwm_bytes_linux() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            // Format: "VmHWM:\t   12345 kB"
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            if kb > 0 {
                return Some(kb * 1024);
            }
        }
    }
    None
}

/// `getrusage(RUSAGE_SELF).ru_maxrss` normalized to bytes. On Linux `ru_maxrss` is in
/// kB; on macOS/BSD it is already in bytes. Returns `0` if the syscall fails.
fn getrusage_maxrss_bytes() -> u64 {
    // SAFETY: `getrusage` fills a caller-owned `rusage`; we zero-initialize it and only
    // read `ru_maxrss` after a successful (`0`) return. `ru_maxrss` is a `c_long` (width
    // varies by platform), so keep it in its native type until the sign check.
    let raw = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
            return 0;
        }
        usage.ru_maxrss
    };
    if raw <= 0 {
        return 0;
    }
    let maxrss = raw as u64;
    #[cfg(target_os = "linux")]
    {
        // Linux reports ru_maxrss in kilobytes.
        maxrss * 1024
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS/BSD report ru_maxrss in bytes.
        maxrss
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_rss_is_plausible_nonzero() {
        let rss = peak_rss();
        // A running test process holds far more than 1 MB resident; any nonzero value in
        // a sane range proves the helper read a real figure (not a stub 0).
        assert!(
            rss > 1024 * 1024,
            "peak_rss too small to be real: {rss} bytes"
        );
        // Loose upper bound well above the container's ~15 GB, guarding a unit mixup
        // (e.g. accidentally returning kB-as-bytes would inflate by 1000x on huge RSS).
        assert!(
            rss < 64u64 * 1024 * 1024 * 1024,
            "peak_rss implausibly large (unit bug?): {rss} bytes"
        );
    }

    #[test]
    fn getrusage_fallback_is_nonzero() {
        // The fallback path alone must also produce a real figure on the supported
        // targets, so SP2's non-Linux / /proc-less path is trustworthy.
        assert!(
            getrusage_maxrss_bytes() > 0,
            "getrusage fallback returned 0"
        );
    }
}
