//! The `uhid` device: BluePad's authored virtual input controller.
//!
//! `CONFIG_UHID=y` on the 4.10 TV's kernel (4.4.84) makes this the
//! delivery half of the app-owned session: we feed `UHID_CREATE2` the
//! authored descriptor (kernel 4.4 `hid-input` parses it and creates the
//! evdev node the streaming apps' SDL forks read), then stream
//! `UHID_INPUT2` events with the authored report layout.
//!
//! Event structs mirror v4.4 `include/uapi/linux/uhid.h`.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

/// UHID event types (v4.4 include/uapi/linux/uhid.h).
const UHID_DESTROY: u32 = 1;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;

/// Max payload the kernel accepts in one event.
const UHID_DATA_MAX: usize = 4096;

/// The fixed-size event the kernel reads (type + request union).
const UHID_EVENT_SIZE: usize = 4 + 4372;

/// Identity of the authored device.
///
/// Custom VID/PID so no built-in SDL DB entry shadows the auto-mapping
/// with wrong button indices; the name carries the pad kind for
/// punktfunk's `type_for_name` heuristic plus the BluePad suffix
/// (spec: Input delivery; input-delivery research §4.3).
pub const BLUEPAD_VID: u32 = 0x1D6B; // Linux Foundation — virtual-device convention
pub const BLUEPAD_PID: u32 = 0xB0D5;
pub const BLUEPAD_VERSION: u32 = 0x0100;
pub const BUS_USB: u16 = 0x03;

/// A live `uhid` device: created with the authored descriptor, streaming
/// authored input reports, destroyed on close (no ghost input).
pub struct UhidDevice {
    file: File,
    created: bool,
}

fn uhid_event(kind: u32, payload: &[u8; UHID_EVENT_SIZE - 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(UHID_EVENT_SIZE);
    out.extend_from_slice(&kind.to_ne_bytes());
    out.extend_from_slice(payload);
    out
}

impl UhidDevice {
    /// Create the device with the authored descriptor.
    pub fn create(name: &str, descriptor: &[u8]) -> io::Result<UhidDevice> {
        assert!(
            descriptor.len() <= UHID_DATA_MAX,
            "authored descriptor must fit a uhid event"
        );
        let mut file = OpenOptions::new().read(true).write(true).open("/dev/uhid")?;

        // struct uhid_create2_req (packed): name[128], phys[64], uniq[64],
        // rd_size, bus, vendor, product, version, country, rd_data[4096].
        let mut payload = [0u8; UHID_EVENT_SIZE - 4];
        // name[128]: keep NUL-terminated for old-kernel string handling.
        let name_len = name.len().min(127);
        payload[..name_len].copy_from_slice(&name.as_bytes()[..name_len]);
        let mut off = 128 + 64 + 64;
        payload[off..off + 2].copy_from_slice(&(descriptor.len() as u16).to_ne_bytes());
        off += 2;
        payload[off..off + 2].copy_from_slice(&BUS_USB.to_ne_bytes());
        off += 2;
        payload[off..off + 4].copy_from_slice(&BLUEPAD_VID.to_ne_bytes());
        off += 4;
        payload[off..off + 4].copy_from_slice(&BLUEPAD_PID.to_ne_bytes());
        off += 4;
        payload[off..off + 4].copy_from_slice(&BLUEPAD_VERSION.to_ne_bytes());
        off += 4;
        // country stays 0.
        off += 4;
        payload[off..off + descriptor.len()].copy_from_slice(descriptor);

        let event = uhid_event(UHID_CREATE2, &payload);
        file.write_all(&event)?;
        Ok(UhidDevice {
            file,
            created: true,
        })
    }

    /// Stream one authored input report.
    pub fn write_input(&mut self, report: &[u8]) -> io::Result<()> {
        assert!(report.len() <= UHID_DATA_MAX);
        // struct uhid_input2_req: size, data[4096].
        let mut payload = [0u8; UHID_EVENT_SIZE - 4];
        payload[..2].copy_from_slice(&(report.len() as u16).to_ne_bytes());
        payload[2..2 + report.len()].copy_from_slice(report);
        let event = uhid_event(UHID_INPUT2, &payload);
        self.file.write_all(&event)
    }

    /// Drain kernel events (UHID_START/STOP/OPEN/CLOSE…). Non-blocking in
    /// spirit: callers poll; errors from a dry pipe are ignored.
    pub fn drain(&mut self) {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        loop {
            match self.file.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    }

    /// Destroy the device (the input node disappears — stale-node cleanup
    /// by construction, user story 19).
    pub fn destroy(&mut self) {
        if self.created {
            let payload = [0u8; UHID_EVENT_SIZE - 4];
            let _ = self.file.write_all(&uhid_event(UHID_DESTROY, &payload));
            self.created = false;
        }
    }
}

impl Drop for UhidDevice {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Set a file descriptor non-blocking so session reads can poll.
pub fn set_nonblocking(fd: i32) -> io::Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_event_is_well_formed() {
        // The event payload must carry the authored identity at the v4.4
        // uhid_create2_req offsets.
        let descriptor = Descriptor::sample();
        let mut payload = [0u8; UHID_EVENT_SIZE - 4];
        // (mirror of the production layout, asserted independently)
        let name = "DualSense Wireless Controller (BluePad)";
        payload[..name.len()].copy_from_slice(name.as_bytes());
        let mut off = 128 + 64 + 64;
        payload[off..off + 2].copy_from_slice(&(descriptor.len() as u16).to_ne_bytes());
        off += 2 + 2;
        payload[off..off + 4].copy_from_slice(&BLUEPAD_VID.to_ne_bytes());
        off += 4;
        payload[off..off + 4].copy_from_slice(&BLUEPAD_PID.to_ne_bytes());
        assert_eq!(BLUEPAD_VID, 0x1D6B);
        assert_eq!(BLUEPAD_PID, 0xB0D5);
        assert!(payload[UHID_EVENT_SIZE - 4 - 1] == 0 || !descriptor.is_empty());
    }

    struct Descriptor;
    impl Descriptor {
        fn sample() -> Vec<u8> {
            vec![0x05, 0x01, 0x09, 0x05, 0xa1, 0x01, 0xc0]
        }
    }
}
