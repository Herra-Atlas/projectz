//! Reading the machine before a job is allowed to run.
//!
//! # Why this is not a crate
//!
//! Three numbers are needed -- RAM use, VRAM use, and how long the machine has
//! been idle -- and none of them is worth a dependency. The Win32 calls that
//! answer them are called directly through `extern`, which also keeps the sampler
//! free of the one thing that would actually cost the machine something: a
//! `sysinfo` refresh walks every process, and shelling out to PowerShell or WMI
//! spawns a process per sample. A job that polls itself twice a minute must not be
//! the reason the PC feels slow.
//!
//! # What is missing, and why it is honest
//!
//! VRAM has no probe here. Reading it means NVML (NVIDIA only), DXGI, or a vendor
//! call, and each is either a dependency or a runtime that may be absent. The
//! design absorbs that: every limit is `Option`, and a figure we cannot read is
//! `None`, which matches *any* threshold. So a job with a VRAM limit runs as if
//! the limit were unset rather than being blocked forever by a number that will
//! never arrive. Wiring a probe in later changes this file only.
//!
//! # Off the UI thread
//!
//! [`sample`] is two syscalls plus a `memcpy` of one fixed-size struct. It is
//! called from the scheduler task, never from a Tauri command, so nothing the
//! user is looking at waits on it.

use serde::Serialize;

use crate::database::jobs::JobConditions;

/// What the machine looks like at one instant.
///
/// `None` is "unknown", never zero: a machine we cannot read must not look idle,
/// which is why these are optional rather than defaulted.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct MachineState {
    /// Percentage of physical memory in use.
    pub ram_percent: Option<f32>,
    /// Percentage of GPU memory in use.
    pub vram_percent: Option<f32>,
    /// Seconds since the last keyboard or mouse input.
    pub idle_seconds: Option<u64>,
}

/// Samples the machine once.
pub fn sample() -> MachineState {
    MachineState {
        ram_percent: ram_percent(),
        // No probe. See the module note: an unreadable figure matches any limit.
        vram_percent: None,
        idle_seconds: idle_seconds(),
    }
}

/// Why a job must not start right now, or `None` when it may.
///
/// Reported as a sentence because it is stored on the run and shown to the user:
/// "RAM 78% is above the 60% limit" answers the question a bare `false` raises.
pub fn start_blocked_by(state: &MachineState, conditions: &JobConditions) -> Option<String> {
    if let Some(reason) = over_limit(state, conditions) {
        return Some(reason);
    }
    if let Some(wanted) = conditions.min_idle_seconds {
        match state.idle_seconds {
            Some(idle) if idle < wanted => {
                return Some(format!(
                    "the machine has been idle {idle}s but the job waits for {wanted}s"
                ));
            }
            // Unknown idle time does not block: a machine we cannot read must not
            // stop a job whose only requirement is that the user is away.
            _ => {}
        }
    }
    None
}

/// Why a *running* job should stop, or `None` when it may continue.
///
/// Only the resource limits, not the idle one: a job that has started may finish
/// even if the user has come back, unless the PC is actually under load. That is
/// the split the stop switch exists for.
pub fn over_limit(state: &MachineState, conditions: &JobConditions) -> Option<String> {
    if let Some(limit) = conditions.max_ram_percent {
        if let Some(ram) = state.ram_percent {
            if ram >= limit {
                return Some(format!("RAM {ram:.0}% is above the {limit:.0}% limit"));
            }
        }
    }
    if let Some(limit) = conditions.max_vram_percent {
        if let Some(vram) = state.vram_percent {
            if vram >= limit {
                return Some(format!("VRAM {vram:.0}% is above the {limit:.0}% limit"));
            }
        }
    }
    None
}

/// Total physical memory, so a percentage can be shown as the bytes it means.
///
/// The editor turns "20%" into "≈6.1 GB" with this. It is the same `memcpy` the
/// sampler already does, not a second source of truth.
#[cfg(windows)]
pub fn ram_total_bytes() -> Option<u64> {
    let mut status = win::MemoryStatusEx {
        length: std::mem::size_of::<win::MemoryStatusEx>() as u32,
        ..Default::default()
    };
    // SAFETY: as in `ram_percent`: a correctly sized, zeroed struct of the exact
    // type the call fills, outliving the call.
    let ok = unsafe { win::GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.total_phys)
}

#[cfg(not(windows))]
pub fn ram_total_bytes() -> Option<u64> {
    None
}

/// Physical memory in use, as a percentage.
#[cfg(windows)]
fn ram_percent() -> Option<f32> {
    let mut status = win::MemoryStatusEx {
        length: std::mem::size_of::<win::MemoryStatusEx>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a correctly sized, zeroed struct of the exact type the
    // call fills, and it outlives the call.
    let ok = unsafe { win::GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.memory_load as f32)
}

#[cfg(not(windows))]
fn ram_percent() -> Option<f32> {
    None
}

/// Seconds since the last user input.
#[cfg(windows)]
fn idle_seconds() -> Option<u64> {
    let mut info = win::LastInputInfo {
        cb_size: std::mem::size_of::<win::LastInputInfo>() as u32,
        dw_time: 0,
    };
    // SAFETY: `info` is the exact struct the call expects, with `cb_size` set as
    // required, and it outlives the call.
    let ok = unsafe { win::GetLastInputInfo(&mut info) };
    if ok == 0 {
        return None;
    }
    // The tick counter wraps every ~49 days, and both figures come from it, so a
    // wrapping subtraction is the correct difference across that boundary --
    // a plain one would report an idle time of nearly fifty years.
    // SAFETY: no arguments, no state.
    let now = unsafe { win::GetTickCount() };
    Some(u64::from(now.wrapping_sub(info.dw_time)) / 1000)
}

#[cfg(not(windows))]
fn idle_seconds() -> Option<u64> {
    None
}

/// The two kernel32 calls this module makes, declared rather than pulled in.
#[cfg(windows)]
mod win {
    #[repr(C)]
    #[derive(Default)]
    pub struct MemoryStatusEx {
        pub length: u32,
        pub memory_load: u32,
        pub total_phys: u64,
        pub avail_phys: u64,
        pub total_page_file: u64,
        pub avail_page_file: u64,
        pub total_virtual: u64,
        pub avail_virtual: u64,
        pub avail_extended_virtual: u64,
    }

    #[repr(C)]
    pub struct LastInputInfo {
        pub cb_size: u32,
        pub dw_time: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
        pub fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
        pub fn GetTickCount() -> u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conditions(max_ram: Option<f32>, min_idle: Option<u64>) -> JobConditions {
        JobConditions {
            max_ram_percent: max_ram,
            max_vram_percent: None,
            min_idle_seconds: min_idle,
            stop_when_busy: true,
        }
    }

    #[test]
    fn a_busy_machine_reports_why_it_blocks() {
        let state = MachineState {
            ram_percent: Some(78.0),
            ..Default::default()
        };
        let reason = start_blocked_by(&state, &conditions(Some(60.0), None)).expect("blocked");
        assert!(reason.contains("RAM"), "{reason}");
    }

    #[test]
    fn an_idle_machine_does_not_block_anything() {
        let state = MachineState {
            ram_percent: Some(10.0),
            vram_percent: Some(5.0),
            idle_seconds: Some(600),
        };
        assert_eq!(start_blocked_by(&state, &conditions(Some(60.0), Some(300))), None);
    }

    /// A figure we cannot read must not block a job: an unreadable GPU would
    /// otherwise hold a VRAM-limited job back forever.
    #[test]
    fn an_unknown_reading_matches_any_limit() {
        let state = MachineState {
            ram_percent: None,
            vram_percent: None,
            idle_seconds: None,
        };
        assert_eq!(
            start_blocked_by(
                &state,
                &JobConditions {
                    max_ram_percent: Some(1.0),
                    max_vram_percent: Some(1.0),
                    min_idle_seconds: Some(3600),
                    stop_when_busy: true,
                }
            ),
            None
        );
    }

    /// The idle requirement is a *start* condition only. A job already running is
    /// not stopped because the user came back -- only actual load stops it.
    #[test]
    fn coming_back_to_the_machine_does_not_stop_a_running_job() {
        let state = MachineState {
            ram_percent: Some(10.0),
            idle_seconds: Some(0),
            ..Default::default()
        };
        assert_eq!(over_limit(&state, &conditions(Some(60.0), Some(300))), None);
    }
}
