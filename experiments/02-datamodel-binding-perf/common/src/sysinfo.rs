//! Small platform helpers for the recorded runs: peak resident memory and a CPU
//! model string. Engine-neutral, kept here so both engine bins share one
//! implementation. These read `/proc` on Linux (the authoritative in-container
//! target, architecture §0) and degrade gracefully elsewhere.

/// Peak resident set size in bytes (`VmHWM` from `/proc/self/status` on Linux). The
/// "high-water mark" is exactly what the §5.4 memory metric wants: the largest RSS
/// the process reached, unaffected by later frees. Returns `0` if unavailable.
pub fn peak_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmHWM:") {
                    // Format: "VmHWM:\t   12345 kB"
                    let kb: u64 = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    return kb * 1024;
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// A best-effort CPU model string (`model name` from `/proc/cpuinfo` on Linux),
/// used to enrich the recorded [`bench_util::Environment`]. Empty if unavailable.
pub fn cpu_model() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(info) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in info.lines() {
                if let Some(rest) = line.strip_prefix("model name") {
                    if let Some((_, v)) = rest.split_once(':') {
                        return v.trim().to_string();
                    }
                }
            }
        }
        String::new()
    }
    #[cfg(not(target_os = "linux"))]
    {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_rss_is_nonzero_on_linux() {
        let rss = peak_rss_bytes();
        #[cfg(target_os = "linux")]
        assert!(rss > 0, "expected a nonzero peak RSS on Linux");
        #[cfg(not(target_os = "linux"))]
        let _ = rss;
    }

    #[test]
    fn cpu_model_is_readable() {
        // Never panics; may be empty off-Linux.
        let _ = cpu_model();
    }
}
