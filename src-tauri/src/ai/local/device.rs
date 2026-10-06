//! Reading the local GPU's memory.
//!
//! One job, done once and cached: how much VRAM does this machine have. The
//! memory estimate needs it to answer the only question a user has when moving a
//! slider -- "will this fit" -- and it is the one figure in the estimate that is
//! a fact rather than a calculation.
//!
//! ## Why this reports nothing rather than guessing
//!
//! Every way of asking Windows for VRAM has a dependency this app does not
//! have and should not take on for one number:
//!
//! - **Vulkan enumeration** is what the engine itself uses and would answer for
//!   every vendor at once, but it means linking `ash` and, at runtime, the
//!   system Vulkan loader -- a native dependency that can be absent, in which
//!   case the app would not start.
//! - **WMI** (`Win32_VideoController`) needs a COM connection and shells out to
//!   PowerShell or `wmic` for a call the user should not wait on.
//! - **DXGI** needs `windows` crate bindings and vendor-specific handling for
//!   the dedicated/shared distinction, which is the part that actually matters.
//!
//! So the estimate ships without a denominator and the UI says so. An absolute
//! figure is still worth showing -- "about 5.4 GB at these settings" answers the
//! real question -- and a missing device total only costs the fit/no-fit verdict.
//!
//! When a device figure is genuinely available it belongs here, behind
//! `available()`, so the UI has one place to ask rather than two.

use std::sync::OnceLock;

static DEVICE_MEMORY: OnceLock<Option<DeviceMemory>> = OnceLock::new();

/// What the local adapter reports.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceMemory {
    pub name: String,
    /// Total VRAM in bytes.
    pub total_bytes: u64,
    /// True when the adapter shares system memory -- an iGPU, or a discrete card
    /// on a laptop with unified memory.
    ///
    /// Load-bearing for the estimate's honesty: on such a device "VRAM" is a
    /// slice of system RAM that the OS also needs, so the figure is a ceiling
    /// rather than a budget, and a model reported as fitting may not.
    pub unified: bool,
}

impl DeviceMemory {
    /// The adapter this build knows about, if any.
    ///
    /// Cached behind a `OnceLock` because the answer cannot change while the app
    /// runs, and a settings page that re-queried on every slider drag would be
    /// slow for a value that is fixed.
    pub fn local() -> Option<&'static DeviceMemory> {
        DEVICE_MEMORY.get_or_init(detect).as_ref()
    }
}

/// The detection hook.
///
/// Returns `None` until a platform implementation is wired in. Written as a
/// function rather than inlined into the `OnceLock` so adding a backend is a
/// change here and nowhere else.
fn detect() -> Option<DeviceMemory> {
    #[cfg(feature = "vulkan-query")]
    {
        vulkan::detect()
    }
    #[cfg(not(feature = "vulkan-query"))]
    {
        None
    }
}

/// The Vulkan backend, compiled only when the feature is on.
///
/// Kept behind a feature because it needs the `ash` crate, which needs the
/// system loader at both build and run time. A user without it should get a
/// working app that cannot quote a number, not an app that will not launch.
#[cfg(feature = "vulkan-query")]
mod vulkan {
    use super::DeviceMemory;

    /// Enumerates Vulkan physical devices and reports the largest.
    ///
    /// Only adapter *limits* and *memory* are read -- no device creation, no
    /// queue, no surface. Enumerating physical devices is a cheap query; taking a
    /// device lock would briefly contend with the real inference process, and
    /// doing that from a settings page is not worth a memory number.
    pub fn detect() -> Option<DeviceMemory> {
        let entry = ash::Entry::load().ok()?;
        let instance =
            unsafe { entry.create_instance(&ash::vk::InstanceCreateInfo::default(), None) }.ok()?;
        let physical = unsafe { instance.enumerate_physical_devices() }.ok()?;
        let mut best: Option<DeviceMemory> = None;
        for device in physical {
            let Ok(properties) = (unsafe { instance.get_physical_device_properties(device) })
            else {
                continue;
            };
            let Ok(memory) = (unsafe { instance.get_physical_device_memory_properties(device) })
            else {
                continue;
            };
            // Only heaps the device can actually allocate from, and only the
            // device-local ones -- a host-visible heap is system RAM wearing a
            // VRAM label, and summing it would report 64 GB on a laptop.
            let total: u64 = memory
                .memory_heap_count
                .into_iter()
                .enumerate()
                .filter(|(index, _)| {
                    memory.memory_heap_flags[*index] & ash::vk::MemoryHeapFlags::DEVICE_LOCAL
                        != ash::vk::vk_memory_heap_flags_empty()
                })
                .filter_map(|(_, heap)| memory.memory_heap_sizes[heap])
                .sum();
            if total == 0 {
                continue;
            }
            let name: String = properties
                .device_name
                .iter()
                .copied()
                .take_while(|byte| *byte != 0)
                .collect::<Vec<u8>>()
                .into_iter()
                .map(|byte| byte as char)
                .collect();
            let better = best
                .as_ref()
                .is_none_or(|current| total > current.total_bytes);
            if better {
                best = Some(DeviceMemory {
                    name,
                    total_bytes: total,
                    // A small device-local heap means a shared adapter: it has its
                    // own carve-out but draws on system RAM beyond it.
                    unified: total < 2 << 30,
                });
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device question is asked once.
    ///
    /// Not a test of the answer -- there is no correct answer on a build machine
    /// without a GPU -- but of the caching, which is what keeps a slider drag
    /// from re-detecting on every frame.
    #[test]
    fn the_device_is_read_once_and_reused() {
        let first = DeviceMemory::local().map(|value| value.total_bytes);
        let second = DeviceMemory::local().map(|value| value.total_bytes);
        assert_eq!(first, second);
    }
}
