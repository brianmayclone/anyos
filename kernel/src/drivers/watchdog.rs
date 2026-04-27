//! Intel i6300ESB Watchdog Timer driver.
//!
//! Provides a hardware watchdog that resets the VM if the kernel stops
//! responding (heartbeat timeout). Emulated by QEMU/KVM.
//!
//! PCI ID: 0x8086:0x25AB

use alloc::boxed::Box;

use crate::drivers::hal::{Driver, DriverError, DriverType};
use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// MMIO virtual address for watchdog registers (after other device regions).
const WDT_MMIO_VIRT: u64 = 0xFFFF_FFFF_D00F_0000;

/// i6300ESB register offsets (MMIO).
const ESB_TIMER1_REG: u64 = 0x00; // Timer 1 value
const ESB_TIMER2_REG: u64 = 0x04; // Timer 2 value
const ESB_RELOAD_REG: u64 = 0x0C; // Reload register (write to pet)
const ESB_CONFIG_REG: u64 = 0x60; // Lock register (PCI config space offset)

/// Unlock sequence: write these two values to RELOAD_REG to unlock timer access.
const ESB_UNLOCK1: u32 = 0x80;
const ESB_UNLOCK2: u32 = 0x86;

/// Timer clock: ~1 MHz (1 tick ≈ 1 µs). 30 seconds = 30_000_000 ticks.
const DEFAULT_TIMEOUT_TICKS: u32 = 30_000_000;

// ── Driver State ─────────────────────────────────────────────────────────────

struct WatchdogState {
    mmio_base: u64,
    enabled: bool,
}

static STATE: Spinlock<Option<WatchdogState>> = Spinlock::new(None);

// ── Public API ──────────────────────────────────────────────────────────────

/// Pet (feed) the watchdog to prevent a reset.
/// Must be called periodically (default: every 30 seconds).
pub fn pet() {
    let state = STATE.lock();
    if let Some(wdt) = state.as_ref() {
        if wdt.enabled {
            // Unlock + write to reload register to restart the countdown.
            unsafe {
                core::ptr::write_volatile(
                    (wdt.mmio_base + ESB_RELOAD_REG) as *mut u32,
                    ESB_UNLOCK1,
                );
                core::ptr::write_volatile(
                    (wdt.mmio_base + ESB_RELOAD_REG) as *mut u32,
                    ESB_UNLOCK2,
                );
            }
        }
    }
}

/// Check if the watchdog is available and enabled.
pub fn is_enabled() -> bool {
    STATE.lock().as_ref().map_or(false, |s| s.enabled)
}

// ── HAL Driver ──────────────────────────────────────────────────────────────

struct WatchdogDriver;

impl Driver for WatchdogDriver {
    fn name(&self) -> &str {
        "i6300ESB Watchdog"
    }
    fn driver_type(&self) -> DriverType {
        DriverType::Char
    }
    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        pet();
        Ok(0)
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

// ── Probe Function ──────────────────────────────────────────────────────────

/// Probe and initialize the i6300ESB watchdog timer.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn Driver>> {
    crate::serial_verbose_println!(
        "Watchdog: probing i6300ESB PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // Get BAR0 (MMIO).
    let bar0 = pci.bars[0] & 0xFFFFFFF0;
    if bar0 == 0 {
        crate::serial_verbose_println!("Watchdog: BAR0 is zero");
        return None;
    }

    // Enable bus mastering + memory access.
    crate::drivers::pci::enable_bus_master(pci);

    // Map MMIO region (1 page is plenty for the register space).
    let phys = PhysAddr::new(bar0 as u64);
    let virt = VirtAddr::new(WDT_MMIO_VIRT);
    virtual_mem::map_page(virt, phys, 0x03); // Present | Writable

    let mmio_base = WDT_MMIO_VIRT;

    // Unlock the timer registers.
    unsafe {
        core::ptr::write_volatile((mmio_base + ESB_RELOAD_REG) as *mut u32, ESB_UNLOCK1);
        core::ptr::write_volatile((mmio_base + ESB_RELOAD_REG) as *mut u32, ESB_UNLOCK2);
    }

    // Set timeout values (Timer1 and Timer2 form a two-stage timeout).
    unsafe {
        core::ptr::write_volatile(
            (mmio_base + ESB_TIMER1_REG) as *mut u32,
            DEFAULT_TIMEOUT_TICKS,
        );
        core::ptr::write_volatile(
            (mmio_base + ESB_TIMER2_REG) as *mut u32,
            DEFAULT_TIMEOUT_TICKS,
        );
    }

    // Enable the watchdog via PCI config space: set bit 1 (timer enable) in config register.
    let config = crate::drivers::pci::pci_config_read32(
        pci.bus,
        pci.device,
        pci.function,
        ESB_CONFIG_REG as u8,
    );
    crate::drivers::pci::pci_config_write32(
        pci.bus,
        pci.device,
        pci.function,
        ESB_CONFIG_REG as u8,
        config | 0x02, // Enable timer
    );

    // Initial pet to start the countdown.
    unsafe {
        core::ptr::write_volatile((mmio_base + ESB_RELOAD_REG) as *mut u32, ESB_UNLOCK1);
        core::ptr::write_volatile((mmio_base + ESB_RELOAD_REG) as *mut u32, ESB_UNLOCK2);
    }

    *STATE.lock() = Some(WatchdogState {
        mmio_base,
        enabled: true,
    });

    crate::serial_verbose_println!("Watchdog: i6300ESB enabled (30s timeout)");

    Some(Box::new(WatchdogDriver))
}
