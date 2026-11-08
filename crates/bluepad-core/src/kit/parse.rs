//! Raw pad report parsing: per-kind report layouts → named control states.
//!
//! Layouts are the primary-source contracts:
//! - DualSense: linux `hid-playstation.c` (`dualsense_input_report`,
//!   `DS_BUTTONS*` masks) and SDL `SDL_hidapi_ps5.c` (simple 0x01 packet).
//! - DualShock 4: `hid-playstation.c` (`dualshock4_input_report_common`,
//!   `DS4_BUTTONS*` masks, `DS4_STATUS0_*`).
//! - Xbox One S: SDL `SDL_hidapi_xboxone.c` Bluetooth packet handlers and
//!   the battery mapping from the GDK reference cited there.

use super::{Button, PadKind};
use serde::{Deserialize, Serialize};

/// Pad battery as parsed from input reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// A parsed pad state: named controls in the authored report's terms.
///
/// Axes are already in the authored 16-bit scale (0..=65535, sticks centered
/// ~0x8000); `hat` is the authored hat ordinal (0=N, 2=E, 4=S, 6=W, 8 neutral).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadInput {
    pub buttons: u16,
    pub hat: u8,
    pub lx: u16,
    pub ly: u16,
    pub l2: u16,
    pub r2: u16,
    pub rx: u16,
    pub ry: u16,
    pub battery: Option<Battery>,
}

impl PadInput {
    pub fn pressed(&self, button: Button) -> bool {
        self.buttons & button.bit() != 0
    }
}

/// One control reading for the pad diagnostic: interpreted label, raw code
/// (where the authored layout knows it), value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    pub label: &'static str,
    pub code: Option<u16>,
    pub value: u16,
}

pub type RawParser = fn(&[u8]) -> Option<PadInput>;

pub fn parser_for(kind: PadKind) -> RawParser {
    match kind {
        PadKind::DualSense => parse_dualsense,
        PadKind::Ds4 => parse_ds4,
        PadKind::XboxOneS => parse_xbox,
        PadKind::GenericHid => |_| None,
    }
}

/// Scale a byte axis (0..=255) to the authored 16-bit scale.
fn scale8(v: u8) -> u16 {
    v as u16 * 257
}

/// Scale an Xbox 10-bit trigger (0..=1023) to the authored 16-bit scale,
/// clamping full-press to exactly 65535 (SDL's convention).
fn scale10(v: u16) -> u16 {
    if v >= 1023 {
        65535
    } else {
        v * 64
    }
}

/// PS battery nibble: 0..=10 capacity, 11 = full (hid-playstation.c).
fn ps_battery_percent(capacity: u8) -> u8 {
    (capacity.min(11) * 10).min(100)
}

/// DualSense button masks (simple and enhanced reports share them):
/// buttons0 = hat (bits 0-3) | square 0x10 | cross 0x20 | circle 0x40 | triangle 0x80.
fn ps_buttons0_buttons(byte: u8) -> (u8, u16) {
    let hat = byte & 0x0F;
    let mut buttons = 0u16;
    if byte & 0x10 != 0 {
        buttons |= Button::Square.bit();
    }
    if byte & 0x20 != 0 {
        buttons |= Button::Cross.bit();
    }
    if byte & 0x40 != 0 {
        buttons |= Button::Circle.bit();
    }
    if byte & 0x80 != 0 {
        buttons |= Button::Triangle.bit();
    }
    (hat, buttons)
}

/// buttons1 = L1 0x01 | R1 0x02 | (digital L2 0x04, R2 0x08 — analog axes
/// carry these, the authored spares 9/10 are not forwarded) | create 0x10 |
/// options 0x20 | L3 0x40 | R3 0x80.
fn ps_buttons1(byte: u8) -> u16 {
    let mut buttons = 0u16;
    if byte & 0x01 != 0 {
        buttons |= Button::L1.bit();
    }
    if byte & 0x02 != 0 {
        buttons |= Button::R1.bit();
    }
    if byte & 0x10 != 0 {
        buttons |= Button::Create.bit();
    }
    if byte & 0x20 != 0 {
        buttons |= Button::Options.bit();
    }
    if byte & 0x40 != 0 {
        buttons |= Button::L3.bit();
    }
    if byte & 0x80 != 0 {
        buttons |= Button::R3.bit();
    }
    buttons
}

/// buttons2 = PS 0x01 (touchpad/mic are out of scope).
fn ps_buttons2(byte: u8) -> u16 {
    let mut buttons = 0u16;
    if byte & 0x01 != 0 {
        buttons |= Button::Guide.bit();
    }
    buttons
}

fn parse_dualsense(raw: &[u8]) -> Option<PadInput> {
    match raw.first()? {
        // Simple Bluetooth report (10 bytes): id, LX, LY, RX, RY,
        // buttons0, buttons1, buttons2, L2, R2 (SDL_hidapi_ps5.c).
        0x01 if raw.len() >= 10 => {
            let (hat, b0) = ps_buttons0_buttons(raw[5]);
            Some(PadInput {
                buttons: b0 | ps_buttons1(raw[6]) | ps_buttons2(raw[7]),
                hat,
                lx: scale8(raw[1]),
                ly: scale8(raw[2]),
                rx: scale8(raw[3]),
                ry: scale8(raw[4]),
                l2: scale8(raw[8]),
                r2: scale8(raw[9]),
                battery: None,
            })
        }
        // Enhanced Bluetooth report 0x31 (78 bytes): id, seq, then the common
        // report (hid-playstation.c `dualsense_input_report`): x, y, rx, ry,
        // z, rz, counter, buttons[4], …, status[0] at byte 54.
        0x31 if raw.len() >= 55 => {
            let (hat, b0) = ps_buttons0_buttons(raw[9]);
            let status0 = raw[54];
            Some(PadInput {
                buttons: b0 | ps_buttons1(raw[10]) | ps_buttons2(raw[11]),
                hat,
                lx: scale8(raw[2]),
                ly: scale8(raw[3]),
                rx: scale8(raw[4]),
                ry: scale8(raw[5]),
                l2: scale8(raw[6]),
                r2: scale8(raw[7]),
                battery: Some(Battery {
                    percent: ps_battery_percent(status0 & 0x0F),
                    charging: status0 & 0xF0 != 0,
                }),
            })
        }
        _ => None,
    }
}

fn parse_ds4(raw: &[u8]) -> Option<PadInput> {
    match raw.first()? {
        // Simple Bluetooth report (10 bytes): id, x, y, rx, ry,
        // buttons0, buttons1, buttons2, z, rz.
        0x01 if raw.len() >= 10 => {
            let (hat, b0) = ps_buttons0_buttons(raw[5]);
            Some(PadInput {
                buttons: b0 | ps_buttons1(raw[6]) | ps_buttons2(raw[7]),
                hat,
                lx: scale8(raw[1]),
                ly: scale8(raw[2]),
                rx: scale8(raw[3]),
                ry: scale8(raw[4]),
                l2: scale8(raw[8]),
                r2: scale8(raw[9]),
                battery: None,
            })
        }
        // Bluetooth report 0x11 (78 bytes): id, 2 reserved bytes, then the
        // common report (hid-playstation.c `dualshock4_input_report_common`):
        // x, y, rx, ry, buttons[3], z, rz, …, status[0] at byte 33.
        0x11 if raw.len() >= 34 => {
            let (hat, b0) = ps_buttons0_buttons(raw[7]);
            let status0 = raw[33];
            Some(PadInput {
                buttons: b0 | ps_buttons1(raw[8]) | ps_buttons2(raw[9]),
                hat,
                lx: scale8(raw[3]),
                ly: scale8(raw[4]),
                rx: scale8(raw[5]),
                ry: scale8(raw[6]),
                l2: scale8(raw[10]),
                r2: scale8(raw[11]),
                battery: Some(Battery {
                    percent: ps_battery_percent(status0 & 0x0F),
                    charging: status0 & 0x10 != 0,
                }),
            })
        }
        _ => None,
    }
}

/// Xbox hat values (1=up … 8=upleft, else neutral) → authored ordinals.
fn xbox_hat(v: u8) -> u8 {
    match v {
        1 => 0, // up
        2 => 1, // up-right
        3 => 2, // right
        4 => 3, // down-right
        5 => 4, // down
        6 => 5, // down-left
        7 => 6, // left
        8 => 7, // up-left
        _ => 8, // neutral
    }
}

fn parse_xbox(raw: &[u8]) -> Option<PadInput> {
    match raw.first()? {
        // Bluetooth state report 0x01 (SDL_hidapi_xboxone.c):
        // LX, LY, RX, RY (LE16, centered 0x8000), LT, RT (LE16, 0-1023),
        // hat, button bytes 14-15.
        0x01 if raw.len() >= 16 => {
            let le16 = |i: usize| u16::from_le_bytes([raw[i], raw[i + 1]]);
            let mut buttons = 0u16;
            // 17-byte (fw 5.x) button layout; the 16-byte variant differs.
            let (byte14, byte15) = if raw.len() >= 17 {
                // A 0x01, B 0x02, X 0x08, Y 0x10, LB 0x40, RB 0x80
                // back 0x04, start 0x08, guide 0x10, L3 0x20, R3 0x40
                (raw[14], raw[15])
            } else {
                // 16-byte: A 0x01, B 0x02, X 0x04, Y 0x08, LB 0x10, RB 0x20,
                // back 0x40, start 0x80; L3 0x01, R3 0x02.
                let mut b = 0u16;
                if raw[14] & 0x01 != 0 {
                    b |= Button::Cross.bit();
                }
                if raw[14] & 0x02 != 0 {
                    b |= Button::Circle.bit();
                }
                if raw[14] & 0x04 != 0 {
                    b |= Button::Square.bit();
                }
                if raw[14] & 0x08 != 0 {
                    b |= Button::Triangle.bit();
                }
                if raw[14] & 0x10 != 0 {
                    b |= Button::L1.bit();
                }
                if raw[14] & 0x20 != 0 {
                    b |= Button::R1.bit();
                }
                if raw[14] & 0x40 != 0 {
                    b |= Button::Create.bit();
                }
                if raw[14] & 0x80 != 0 {
                    b |= Button::Options.bit();
                }
                if raw[15] & 0x01 != 0 {
                    b |= Button::L3.bit();
                }
                if raw[15] & 0x02 != 0 {
                    b |= Button::R3.bit();
                }
                return Some(PadInput {
                    buttons: b,
                    hat: xbox_hat(raw[13]),
                    lx: le16(1),
                    ly: le16(3),
                    rx: le16(5),
                    ry: le16(7),
                    l2: scale10(le16(9)),
                    r2: scale10(le16(11)),
                    battery: None,
                });
            };
            if byte14 & 0x01 != 0 {
                buttons |= Button::Cross.bit();
            }
            if byte14 & 0x02 != 0 {
                buttons |= Button::Circle.bit();
            }
            if byte14 & 0x08 != 0 {
                buttons |= Button::Square.bit();
            }
            if byte14 & 0x10 != 0 {
                buttons |= Button::Triangle.bit();
            }
            if byte14 & 0x40 != 0 {
                buttons |= Button::L1.bit();
            }
            if byte14 & 0x80 != 0 {
                buttons |= Button::R1.bit();
            }
            if byte15 & 0x04 != 0 {
                buttons |= Button::Create.bit();
            }
            if byte15 & 0x08 != 0 {
                buttons |= Button::Options.bit();
            }
            if byte15 & 0x10 != 0 {
                buttons |= Button::Guide.bit();
            }
            if byte15 & 0x20 != 0 {
                buttons |= Button::L3.bit();
            }
            if byte15 & 0x40 != 0 {
                buttons |= Button::R3.bit();
            }
            Some(PadInput {
                buttons,
                hat: xbox_hat(raw[13]),
                lx: le16(1),
                ly: le16(3),
                rx: le16(5),
                ry: le16(7),
                l2: scale10(le16(9)),
                r2: scale10(le16(11)),
                battery: None,
            })
        }
        // Battery report 0x04: flags byte 1 — capacity bits 0-1 → 10/40/70/100,
        // bits 2-3 zero → on USB (SDL_hidapi_xboxone.c,
        // HIDAPI_DriverXboxOne_HandleBatteryState).
        0x04 if raw.len() >= 2 => {
            let flags = raw[1];
            let percent = match flags & 0x03 {
                0 => 10,
                1 => 40,
                2 => 70,
                _ => 100,
            };
            Some(PadInput {
                battery: Some(Battery {
                    percent,
                    charging: flags & 0x0C == 0,
                }),
                ..PadInput::default()
            })
        }
        _ => None,
    }
}
