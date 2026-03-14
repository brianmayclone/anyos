//! Network backend: TAP device for connecting E1000 to the host network.
//!
//! On Linux, creates a TAP device and bridges it with the host network.
//! The TAP device is a layer-2 (Ethernet) tunnel that allows raw frame
//! exchange between the VM and the host.

#[cfg(feature = "linux")]
pub mod tap {
    use std::io;
    use std::os::unix::io::{RawFd, AsRawFd};

    // ioctl constants for TUN/TAP
    const TUNSETIFF: u64 = 0x400454CA;
    const IFF_TAP: i16 = 0x0002;
    const IFF_NO_PI: i16 = 0x1000;

    /// A Linux TAP network device.
    pub struct TapDevice {
        fd: RawFd,
        name: String,
    }

    // ifreq struct layout for TUNSETIFF
    #[repr(C)]
    struct Ifreq {
        ifr_name: [u8; 16],
        ifr_flags: i16,
        _pad: [u8; 22],
    }

    impl TapDevice {
        /// Create and open a new TAP device.
        /// If `name` is empty, the kernel assigns a name (tap0, tap1, ...).
        /// Requires CAP_NET_ADMIN or root.
        pub fn new(name: &str) -> io::Result<Self> {
            // Open /dev/net/tun
            let fd = unsafe {
                libc::open(b"/dev/net/tun\0".as_ptr() as *const libc::c_char,
                           libc::O_RDWR | libc::O_NONBLOCK)
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }

            // Configure as TAP (layer 2) without packet info header
            let mut ifr = Ifreq {
                ifr_name: [0u8; 16],
                ifr_flags: IFF_TAP | IFF_NO_PI,
                _pad: [0u8; 22],
            };

            // Copy device name (truncate to 15 chars + null)
            let name_bytes = name.as_bytes();
            let copy_len = name_bytes.len().min(15);
            ifr.ifr_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

            let ret = unsafe {
                libc::ioctl(fd, TUNSETIFF as libc::c_ulong, &ifr as *const Ifreq)
            };
            if ret < 0 {
                unsafe { libc::close(fd); }
                return Err(io::Error::last_os_error());
            }

            // Read back the assigned name
            let name_end = ifr.ifr_name.iter().position(|&b| b == 0).unwrap_or(16);
            let assigned_name = String::from_utf8_lossy(&ifr.ifr_name[..name_end]).to_string();

            eprintln!("[net] TAP device '{}' opened (fd={})", assigned_name, fd);

            Ok(TapDevice {
                fd,
                name: assigned_name,
            })
        }

        /// Get the TAP device name (e.g. "tap0").
        pub fn name(&self) -> &str {
            &self.name
        }

        /// Read an Ethernet frame from the TAP device.
        /// Returns the number of bytes read, or 0 if no data available (non-blocking).
        pub fn read_frame(&self, buf: &mut [u8]) -> io::Result<usize> {
            let n = unsafe {
                libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    return Ok(0);
                }
                return Err(err);
            }
            Ok(n as usize)
        }

        /// Write an Ethernet frame to the TAP device.
        pub fn write_frame(&self, data: &[u8]) -> io::Result<usize> {
            let n = unsafe {
                libc::write(self.fd, data.as_ptr() as *const libc::c_void, data.len())
            };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(n as usize)
        }

        /// Bring the TAP interface up using a system call.
        pub fn bring_up(&self) -> io::Result<()> {
            let status = std::process::Command::new("ip")
                .args(["link", "set", &self.name, "up"])
                .status()
                .map_err(|e| io::Error::new(io::ErrorKind::Other,
                    format!("Failed to run 'ip link set {} up': {}", self.name, e)))?;
            if !status.success() {
                return Err(io::Error::new(io::ErrorKind::Other,
                    format!("'ip link set {} up' failed with {}", self.name, status)));
            }
            Ok(())
        }

        /// Add the TAP interface to an existing bridge.
        pub fn add_to_bridge(&self, bridge: &str) -> io::Result<()> {
            let status = std::process::Command::new("ip")
                .args(["link", "set", &self.name, "master", bridge])
                .status()
                .map_err(|e| io::Error::new(io::ErrorKind::Other,
                    format!("Failed to add {} to bridge {}: {}", self.name, bridge, e)))?;
            if !status.success() {
                return Err(io::Error::new(io::ErrorKind::Other,
                    format!("Bridge add failed: {} → {}", self.name, bridge)));
            }
            Ok(())
        }
    }

    impl AsRawFd for TapDevice {
        fn as_raw_fd(&self) -> RawFd {
            self.fd
        }
    }

    impl Drop for TapDevice {
        fn drop(&mut self) {
            eprintln!("[net] Closing TAP device '{}'", self.name);
            unsafe { libc::close(self.fd); }
        }
    }

    /// Poll the TAP device and deliver received frames to the E1000 NIC.
    /// Also take transmitted frames from E1000 and send them to the TAP device.
    /// Returns the number of packets processed (rx + tx).
    pub fn poll_network(tap: &TapDevice, vm_handle: u64) -> u32 {
        let mut count: u32 = 0;

        // 1. Deliver host→guest packets (TAP → E1000 RX)
        let mut rx_buf = [0u8; 2048]; // Max Ethernet frame
        loop {
            match tap.read_frame(&mut rx_buf) {
                Ok(0) => break, // No more data
                Ok(n) => {
                    crate::ffi::corevm_e1000_receive(vm_handle, rx_buf.as_ptr(), n as u32);
                    count += 1;
                }
                Err(_) => break,
            }
        }

        // 2. Send guest→host packets (E1000 TX → TAP)
        let mut tx_buf = [0u8; 65536];
        let tx_count = crate::ffi::corevm_e1000_take_tx(vm_handle, tx_buf.as_mut_ptr(), tx_buf.len() as u32);
        if tx_count > 0 {
            let mut offset = 0;
            for _ in 0..tx_count {
                if offset + 2 > tx_buf.len() { break; }
                let pkt_len = (tx_buf[offset] as usize) | ((tx_buf[offset + 1] as usize) << 8);
                offset += 2;
                if offset + pkt_len > tx_buf.len() { break; }
                let _ = tap.write_frame(&tx_buf[offset..offset + pkt_len]);
                offset += pkt_len;
                count += 1;
            }
        }

        count
    }
}
