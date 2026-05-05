//! Display output / scanout data model.
//!
//! Canonical types shared between the kernel display subsystem, the
//! `GpuDriver` trait, the new display syscalls, and (via mirroring FFI
//! definitions) user-space `libdisplay_client`.
//!
//! Naming follows the DRM/KMS convention so that the model maps 1:1
//! onto the abstractions readers may already know:
//!
//! - [`OutputMode`] ≈ `drm_display_mode` (resolution + refresh + bpp)
//! - [`OutputInfo`] ≈ a connector's reported state (current mode,
//!   modeset list, EDID-derived metadata)
//! - [`OutputLayoutEntry`] ≈ a CRTC + plane assignment (where the
//!   output sits in the virtual desktop, scale, mirror parent)
//! - [`OutputLayout`] is the *atomic commit object* — submit it whole
//!   or not at all.
//!
//! Refresh rate is stored in **millihertz** so 59.94 Hz round-trips
//! exactly (59940 mHz), mirroring DRM's `drm_display_mode.vrefresh`.

use alloc::vec::Vec;

/// Maximum number of scanouts per GPU device.
///
/// Matches the virtio-gpu specification (`num_scanouts` is constrained
/// to 1..=16). Hardware GPU drivers (Intel, AMD, Nvidia) seldom expose
/// more than 8 outputs simultaneously; 16 is plenty of headroom.
pub const MAX_OUTPUTS: usize = 16;

/// A specific scanout configuration: resolution + refresh + bit depth.
///
/// Refresh is in millihertz (`60_000` = 60 Hz, `59_940` = 59.94 Hz).
/// `0` means unknown / driver-default.
///
/// `bpp` is the *bits per pixel*. anyOS currently only uses 32 bpp
/// BGRA but the field is reserved so future 16-bit or 10-bit-per-channel
/// modes can be represented without a model change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    pub bpp: u8,
}

impl OutputMode {
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            refresh_mhz: 60_000,
            bpp: 32,
        }
    }

    /// Pixel pitch for a tightly-packed framebuffer at this mode.
    pub const fn pitch(&self) -> u32 {
        // bpp/8, rounded up
        self.width * ((self.bpp as u32 + 7) / 8)
    }
}

/// Reported state of a single physical output (scanout).
///
/// Populated by the GPU driver from `GET_DISPLAY_INFO` + `GET_EDID`.
/// Kept in the kernel; user space sees a flattened FFI version through
/// the `SYS_DISPLAY_LIST` syscall.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    /// Stable per-boot scanout index (0..MAX_OUTPUTS-1).
    pub id: u32,

    /// True when the host indicates a monitor is attached.
    /// On QEMU this is essentially "scanout enabled in GET_DISPLAY_INFO";
    /// on real hardware this tracks HPD (hot-plug-detect) state.
    pub connected: bool,

    /// The mode currently being scanned out, if any. `None` when the
    /// scanout is disabled (resource_id 0) or no mode has been set yet.
    pub current_mode: Option<OutputMode>,

    /// The display's preferred mode (typically EDID detailed timing #1
    /// or, on QEMU virtio-gpu, the geometry reported by GET_DISPLAY_INFO).
    pub preferred_mode: Option<OutputMode>,

    /// All modes the driver is willing to set on this output.
    pub modes: Vec<OutputMode>,

    /// Physical screen size in millimetres, as reported by EDID.
    /// `(0, 0)` if EDID was not readable or the field was absent.
    pub physical_mm: (u16, u16),

    /// CRC-style hash of the full 128- or 256-byte EDID block.
    /// `0` if no EDID was readable. Used by `displayd` to identify the
    /// same monitor across hotplug events even if the scanout index
    /// changes.
    pub edid_hash: u64,

    /// 3-letter PNPID from EDID bytes 8-9 (e.g. `b"DEL"` for Dell),
    /// null-terminated. `[0; 4]` if EDID is unavailable.
    pub manufacturer: [u8; 4],
}

impl OutputInfo {
    /// Construct a minimal info entry for a scanout that the driver
    /// knows about but has not yet probed deeply (used as a starting
    /// point that EDID/mode-list queries fill in).
    pub fn placeholder(id: u32) -> Self {
        Self {
            id,
            connected: false,
            current_mode: None,
            preferred_mode: None,
            modes: Vec::new(),
            physical_mm: (0, 0),
            edid_hash: 0,
            manufacturer: [0; 4],
        }
    }
}

/// One scanout's slot in a layout proposal.
///
/// `mirror_of` lets a layout describe true display mirroring: the
/// referenced output's resource_id is reused (per the virtio-gpu spec
/// "shared backing store" model), so no duplicate guest RAM is needed.
/// A mirror entry's `mode.width/height` must match the source output's
/// mode (the layout validator enforces this before applying).
#[derive(Debug, Clone)]
pub struct OutputLayoutEntry {
    pub id: u32,

    /// Position and size of this output in the global virtual desktop.
    /// `(width, height)` always equals the chosen mode's `(width, height)`
    /// scaled by `scale/100` (already accounted for so the compositor
    /// never has to recompute it).
    pub virtual_x: i32,
    pub virtual_y: i32,
    pub virtual_w: u32,
    pub virtual_h: u32,

    /// The mode to set on this output.
    pub mode: OutputMode,

    /// HiDPI scaling, in percent. 100 = 1.0x (native), 200 = 2.0x.
    /// The compositor multiplies logical sizes by `scale/100` when
    /// laying out windows on this output.
    pub scale: u16,

    /// `Some(other_id)` if this output should mirror `other_id`.
    /// `None` for an independently driven output.
    pub mirror_of: Option<u32>,

    /// Exactly one entry in a layout must be marked primary; new
    /// windows without an explicit output preference are placed there.
    pub primary: bool,
}

/// A complete proposed display configuration.
///
/// Submitted to the kernel as one atomic unit. The kernel's
/// [`apply_layout`](crate::drivers::gpu::GpuDriver::apply_layout)
/// implementation:
///
///  1. Validates each entry (output id in range, mode supported,
///     mirror chains acyclic, mirror modes match source modes,
///     exactly one primary).
///  2. If validation passes, walks the entries and updates each
///     scanout — in an order that minimises visible glitches
///     (disable removed outputs first, then reconfigure existing,
///     then enable newly added).
///  3. Returns success/error to the caller. On error the previous
///     layout is left untouched.
///
/// This mirrors how DRM atomic commits work and lets `displayd` /
/// the display-settings app preview a layout, react to a `TEST_ONLY`
/// validation pass, and commit only when the user hits Apply.
#[derive(Debug, Clone)]
pub struct OutputLayout {
    pub entries: Vec<OutputLayoutEntry>,
}

impl OutputLayout {
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Find the entry for the primary output, if the layout has one.
    pub fn primary(&self) -> Option<&OutputLayoutEntry> {
        self.entries.iter().find(|e| e.primary)
    }

    /// Validate the layout against a list of physically present outputs.
    ///
    /// Returns `Ok(())` if the layout is internally consistent and every
    /// entry references an existing output that is currently connected.
    /// Returns a stable error code so user-space tooling can localise
    /// the message.
    pub fn validate(&self, present: &[OutputInfo]) -> Result<(), LayoutError> {
        // Exactly one primary.
        let primary_count = self.entries.iter().filter(|e| e.primary).count();
        if primary_count != 1 {
            return Err(LayoutError::PrimaryCount(primary_count));
        }

        // No duplicate ids in the proposal.
        for i in 0..self.entries.len() {
            for j in (i + 1)..self.entries.len() {
                if self.entries[i].id == self.entries[j].id {
                    return Err(LayoutError::DuplicateOutput(self.entries[i].id));
                }
            }
        }

        for entry in &self.entries {
            // Output must exist and be connected.
            let info = present
                .iter()
                .find(|o| o.id == entry.id)
                .ok_or(LayoutError::UnknownOutput(entry.id))?;
            if !info.connected {
                return Err(LayoutError::OutputDisconnected(entry.id));
            }

            // Mode must be in the supported list (or the modes list may
            // be empty for drivers that don't enumerate, in which case
            // any non-zero mode is accepted optimistically).
            if !info.modes.is_empty()
                && !info
                    .modes
                    .iter()
                    .any(|m| m.width == entry.mode.width && m.height == entry.mode.height)
            {
                return Err(LayoutError::ModeUnsupported {
                    output: entry.id,
                    width: entry.mode.width,
                    height: entry.mode.height,
                });
            }

            // Scale must be in a sane range.
            if entry.scale < 50 || entry.scale > 400 {
                return Err(LayoutError::ScaleOutOfRange(entry.scale));
            }

            // Virtual rect must be non-zero.
            if entry.virtual_w == 0 || entry.virtual_h == 0 {
                return Err(LayoutError::ZeroVirtualRect(entry.id));
            }

            // Mirror target (if any) must exist in the same proposal,
            // must not point at self, and chain depth must not exceed 1.
            if let Some(target) = entry.mirror_of {
                if target == entry.id {
                    return Err(LayoutError::MirrorSelf(entry.id));
                }
                let parent = self.entries.iter().find(|e| e.id == target).ok_or(
                    LayoutError::MirrorTargetMissing {
                        output: entry.id,
                        target,
                    },
                )?;
                if parent.mirror_of.is_some() {
                    return Err(LayoutError::MirrorChain(entry.id));
                }
                // Mirror modes must match the source so a single resource
                // can satisfy both scanouts.
                if parent.mode.width != entry.mode.width || parent.mode.height != entry.mode.height
                {
                    return Err(LayoutError::MirrorModeMismatch {
                        output: entry.id,
                        target,
                    });
                }
            }
        }

        Ok(())
    }
}

/// Stable error codes for layout validation. User-space callers map
/// these to localised strings; the kernel never formats the messages
/// itself to keep `no_std` compatibility simple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    /// `count` outputs were marked primary; exactly one is required.
    PrimaryCount(usize),
    DuplicateOutput(u32),
    UnknownOutput(u32),
    OutputDisconnected(u32),
    ModeUnsupported {
        output: u32,
        width: u32,
        height: u32,
    },
    ScaleOutOfRange(u16),
    ZeroVirtualRect(u32),
    MirrorSelf(u32),
    MirrorTargetMissing {
        output: u32,
        target: u32,
    },
    /// Mirror entries pointing at a mirror entry — chain depth > 1.
    MirrorChain(u32),
    MirrorModeMismatch {
        output: u32,
        target: u32,
    },
}

impl LayoutError {
    /// Stable numeric code for FFI / syscall return values.
    pub const fn code(&self) -> u32 {
        match self {
            LayoutError::PrimaryCount(_) => 1,
            LayoutError::DuplicateOutput(_) => 2,
            LayoutError::UnknownOutput(_) => 3,
            LayoutError::OutputDisconnected(_) => 4,
            LayoutError::ModeUnsupported { .. } => 5,
            LayoutError::ScaleOutOfRange(_) => 6,
            LayoutError::ZeroVirtualRect(_) => 7,
            LayoutError::MirrorSelf(_) => 8,
            LayoutError::MirrorTargetMissing { .. } => 9,
            LayoutError::MirrorChain(_) => 10,
            LayoutError::MirrorModeMismatch { .. } => 11,
        }
    }
}

/// CRC-64 (ECMA polynomial `0x42F0E1EBA9EA3693`) of an EDID block.
///
/// Used by `displayd` to identify the same monitor across hotplug events
/// even if the scanout index changes between connect / disconnect cycles.
/// The choice of polynomial matches what `edid-decode` and the Linux DRM
/// core use, which keeps cross-tool comparisons trivial.
///
/// Implementation is a small bit-by-bit MSB-first CRC; an EDID is at most
/// 256 bytes so a table is overkill. Returns `0` for an empty input
/// (an explicit sentinel for "no EDID readable" used elsewhere in the
/// display stack).
pub fn edid_hash(edid: &[u8]) -> u64 {
    if edid.is_empty() {
        return 0;
    }
    const POLY: u64 = 0x42F0_E1EB_A9EA_3693;
    let mut crc: u64 = 0;
    for &byte in edid {
        crc ^= (byte as u64) << 56;
        for _ in 0..8 {
            if crc & (1u64 << 63) != 0 {
                crc = (crc << 1) ^ POLY;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Hotplug / configuration-change notifications produced by the kernel
/// display subsystem. Drained by the compositor via `SYS_DISPLAY_POLL_EVENT`.
///
/// The compositor forwards these over IPC to `displayd`, which decides
/// what (if anything) to do about them and may push back a fresh
/// `OutputLayout` via `CMD_APPLY_LAYOUT`.
#[derive(Debug, Clone, Copy)]
pub enum DisplayEvent {
    /// One or more outputs changed connection state. The receiver should
    /// re-query `SYS_DISPLAY_LIST` rather than relying on diff hints
    /// here — keeps the event payload tiny and avoids racing with
    /// further changes that may arrive between the event and the read.
    HotplugChanged,

    /// An output's preferred mode changed (e.g. the host resized the
    /// QEMU window with a SPICE viewer attached → vdagent monitors-config).
    PreferredModeChanged { output: u32 },

    /// The kernel applied a new layout — emitted *after* a successful
    /// `apply_layout` so observers can update their state mirrors.
    LayoutApplied,
}
