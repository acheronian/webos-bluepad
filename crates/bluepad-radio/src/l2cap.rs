//! The app-owned session: raw kernel L2CAP + `uhid`.
//!
//! This is the revision-agnostic path: BluePad opens the HID channels to
//! the pad itself, parses the reports with the per-kind kit, and creates
//! the input device through `/dev/uhid` with the authored descriptor —
//! bypassing the TV's HID handling entirely (spec: Connection strategy).
//!
//! **On-device gate:** the 4.10 TV's kernel has no kernel Bluetooth (all
//! BT lives in LG's userspace), so this transport serves the onboard radio
//! only if the feasibility spike (issue #12) proves a route in; it serves
//! a kernel-BT USB dongle as-is. The session shape below is the one the
//! spike validates: control PSM 0x11, interrupt PSM 0x13.

use bluepad_core::bdaddr::BdAddr;
use bluepad_core::kit::{PadInput, PadKit, PadKitFactory};
use bluepad_core::pad::{ConnectionPath, PadIdentity};
use bluepad_core::transport::{HidSession, TransportError};
use crate::uhid::{set_nonblocking, UhidDevice};
use std::io;
use std::time::{Duration, Instant};

// Kernel L2CAP socket UAPI (linux include/uapi/linux/bluetooth.h) — the
// libc crate does not ship Bluetooth structs.
const AF_BLUETOOTH: i32 = 31;
const BTPROTO_L2CAP: i32 = 0;
const SOCK_SEQPACKET: i32 = 5;

#[repr(C)]
struct SockaddrL2 {
    l2_family: u16,
    /// __le16: the kernel expects little-endian; targets are LE.
    l2_psm: u16,
    l2_bdaddr: [u8; 6],
    l2_cid: u16,
    l2_bdaddr_type: u8,
}

/// HID control channel.
pub const PSM_HID_CONTROL: u16 = 0x11;
/// HID interrupt channel (input and output reports).
pub const PSM_HID_INTERRUPT: u16 = 0x13;

/// A session over the raw L2CAP HID channels, streaming into a `uhid`
/// device built from the authored descriptor.
pub struct L2capSession {
    control: i32,
    interrupt: i32,
    uhid: UhidDevice,
    kit: PadKit,
    last_input: Option<Instant>,
    _identity: PadIdentity,
}

fn l2cap_connect(address: &BdAddr, psm: u16) -> io::Result<i32> {
    let fd = unsafe { libc::socket(AF_BLUETOOTH, SOCK_SEQPACKET, BTPROTO_L2CAP) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let bytes = address.bytes();
    let addr = SockaddrL2 {
        l2_family: AF_BLUETOOTH as u16,
        l2_psm: psm,
        l2_bdaddr: bytes,
        l2_cid: 0,
        l2_bdaddr_type: 0, // BR/EDR
    };
    let ret = unsafe {
        libc::connect(
            fd,
            &addr as *const SockaddrL2 as *const libc::sockaddr,
            std::mem::size_of::<SockaddrL2>() as u32,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }
    set_nonblocking(fd)?;
    Ok(fd)
}

impl L2capSession {
    /// Open both HID channels to a paired pad and create the authored
    /// `uhid` device.
    pub fn open(identity: &PadIdentity) -> Result<L2capSession, TransportError> {
        let kind = identity.kind();
        let Some(kit) = bluepad_core::kit::factory().kit(kind, ConnectionPath::AppOwned) else {
            return Err(TransportError::new(
                bluepad_core::failure::FailureReason::UnsupportedCombination,
                format!("{kind:?} has no app-owned kit"),
            ));
        };
        let control = l2cap_connect(&identity.address, PSM_HID_CONTROL).map_err(|e| {
            TransportError::new(
                bluepad_core::failure::FailureReason::PadUnreachable,
                format!("L2CAP control channel: {e}"),
            )
        })?;
        let interrupt = l2cap_connect(&identity.address, PSM_HID_INTERRUPT).map_err(|e| {
            unsafe { libc::close(control) };
            TransportError::new(
                bluepad_core::failure::FailureReason::PadUnreachable,
                format!("L2CAP interrupt channel: {e}"),
            )
        })?;
        // The authored device name carries the kind (punktfunk's
        // name heuristic) plus the BluePad suffix.
        let name = format!("{} (BluePad)", kind.label());
        let uhid = UhidDevice::create(&name, kit.descriptor()).map_err(|e| {
            unsafe {
                libc::close(control);
                libc::close(interrupt);
            }
            TransportError::new(
                bluepad_core::failure::FailureReason::InternalError,
                format!("uhid create: {e}"),
            )
        })?;
        Ok(L2capSession {
            control,
            interrupt,
            uhid,
            kit,
            last_input: None,
            _identity: identity.clone(),
        })
    }
}

impl HidSession for L2capSession {
    fn path(&self) -> ConnectionPath {
        ConnectionPath::AppOwned
    }

    fn poll_input(&mut self) -> Result<Option<PadInput>, TransportError> {
        let mut buf = [0u8; 78 + 4];
        let n = unsafe {
            libc::read(
                self.interrupt,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            return match err.kind() {
                io::ErrorKind::WouldBlock => Ok(None),
                _ => Err(TransportError::new(
                    bluepad_core::failure::FailureReason::SessionLost,
                    format!("interrupt read: {err}"),
                )),
            };
        }
        if n == 0 {
            return Err(TransportError::new(
                bluepad_core::failure::FailureReason::SessionLost,
                "pad closed the interrupt channel",
            ));
        }
        let raw = &buf[..n as usize];
        let Some(input) = self.kit.parse_raw(raw) else {
            return Ok(None); // report id we don't decode (sensors etc.)
        };
        // Forward through the authored descriptor.
        let authored = self.kit.encode_authored_report(&input);
        self.uhid
            .write_input(&authored)
            .map_err(|e| {
                TransportError::new(
                    bluepad_core::failure::FailureReason::InternalError,
                    format!("uhid input: {e}"),
                )
            })?;
        self.last_input = Some(Instant::now());
        Ok(Some(input))
    }

    fn write_output(&mut self, report: &[u8]) -> Result<(), TransportError> {
        // Output reports ride the interrupt channel (HID-over-L2CAP).
        let n = unsafe {
            libc::write(
                self.interrupt,
                report.as_ptr() as *const libc::c_void,
                report.len(),
            )
        };
        if n < 0 {
            return Err(TransportError::new(
                bluepad_core::failure::FailureReason::SessionLost,
                format!("interrupt write: {}", io::Error::last_os_error()),
            ));
        }
        Ok(())
    }

    fn input_flowing(&self) -> bool {
        match self.last_input {
            Some(at) => at.elapsed() <= SILENCE_LIMIT,
            None => false,
        }
    }

    fn close(&mut self) -> Result<(), TransportError> {
        // The uhid device is destroyed first (Drop also guards), then the
        // channels: the input node never outlives the session.
        self.uhid.destroy();
        unsafe {
            libc::close(self.control);
            libc::close(self.interrupt);
        }
        Ok(())
    }
}

/// Silence past which the session counts as blacked out — must match the
/// supervisor's SILENCE_LIMIT contract.
const SILENCE_LIMIT: Duration = Duration::from_secs(10);

/// The app-owned transport: opens sessions on demand. Pad-initiated
/// accepts (wake, re-advertise) require a listening PSM — gated on the
/// on-device spike, since registering HID PSMs competes with whatever BT
/// userspace owns the radio.
pub struct L2capTransport;

impl L2capTransport {
    pub fn new() -> L2capTransport {
        L2capTransport
    }
}

impl Default for L2capTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl bluepad_core::transport::AppOwnedTransport for L2capTransport {
    fn accept_incoming(&mut self) -> Result<Option<PadIdentity>, TransportError> {
        // Requires a listening socket on the HID PSMs; the spike decides
        // whether that is possible against the TV's BT userspace.
        Ok(None)
    }

    fn connect(
        &mut self,
        identity: &PadIdentity,
    ) -> Result<Box<dyn HidSession>, TransportError> {
        L2capSession::open(identity).map(|s| Box::new(s) as Box<dyn HidSession>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_psms_are_the_profile_constants() {
        assert_eq!(PSM_HID_CONTROL, 0x11);
        assert_eq!(PSM_HID_INTERRUPT, 0x13);
    }
}
