//! Kit seam tests: the authored HID descriptor → expected kernel codes.
//!
//! Ground truth is the kernel 4.4 `hid-input` gamepad contract (documented in
//! `docs/research/streaming-app-input-delivery.md` §4.2, verified against linux
//! v4.4 `drivers/hid/hid-input.c`): Gamepad-application button *n* →
//! `BTN_GAMEPAD + (n-1)`, D-pad usages → `ABS_HAT0X`/`ABS_HAT0Y`, stick usages
//! → the ABS code of the matching Generic Desktop usage.
//!
//! The test walks the descriptor bytes with its own independent mini-parser, so
//! a bug in the builder cannot be hidden by a bug in a shared parser.

use bluepad_core::kit::PadKitFactory;
use bluepad_core::kit::{self, Button, PadKit, PadInput};
use bluepad_core::{ConnectionPath, PadKind};

/// Kernel 4.4 input-event codes the authored descriptor must land on
/// (include/uapi/linux/input-event-codes.h, v4.4).
mod codes {
    pub const ABS_X: u16 = 0x00;
    pub const ABS_Y: u16 = 0x01;
    pub const ABS_Z: u16 = 0x02;
    pub const ABS_RX: u16 = 0x03;
    pub const ABS_RY: u16 = 0x04;
    pub const ABS_RZ: u16 = 0x05;
    pub const ABS_HAT0X: u16 = 0x10;
    pub const ABS_HAT0Y: u16 = 0x11;
    pub const BTN_SOUTH: u16 = 0x130;
    pub const BTN_EAST: u16 = 0x131;
    pub const BTN_C: u16 = 0x132;
    pub const BTN_NORTH: u16 = 0x133;
    pub const BTN_WEST: u16 = 0x134;
    pub const BTN_Z: u16 = 0x135;
    pub const BTN_TL: u16 = 0x136;
    pub const BTN_TR: u16 = 0x137;
    pub const BTN_TL2: u16 = 0x138;
    pub const BTN_TR2: u16 = 0x139;
    pub const BTN_SELECT: u16 = 0x13a;
    pub const BTN_START: u16 = 0x13b;
    pub const BTN_MODE: u16 = 0x13c;
    pub const BTN_THUMBL: u16 = 0x13d;
    pub const BTN_THUMBR: u16 = 0x13e;
}

/// What the independent descriptor walk found: the usages of the Gamepad
/// application collection, in report order.
struct Walked {
    report_id: u8,
    axes: Vec<(u16, u16)>, // (usage id, bits per field)
    hat: Option<u16>,      // bits of the hat field
    buttons: Option<(u16, u16, u16)>, // (usage min, usage max, bits per button)
    input_bits: usize,     // total input report bits, padding included
}

/// Minimal independent HID descriptor walker: enough to recover the usages,
/// field sizes and report id of the input report. Deliberately shares no code
/// with the builder.
fn walk(descriptor: &[u8]) -> Walked {
    let mut out = Walked {
        report_id: 0,
        axes: vec![],
        hat: None,
        buttons: None,
        input_bits: 0,
    };
    let mut report_id = 0u8;
    let mut usage_page = 0u16;
    let mut usages: Vec<u16> = Vec::new();
    let mut usage_min = 0u16;
    let mut usage_max = 0u16;
    let mut report_size = 0usize;
    let mut report_count = 0usize;
    let mut depth = 0usize;
    let mut i = 0;
    while i < descriptor.len() {
        let b = descriptor[i];
        i += 1;
        if b == 0xFE {
            // Long item: 0xFE, data length, long-item tag, data.
            i += 1 + descriptor[i] as usize + 1;
            continue;
        }
        let size = match b & 0x03 {
            3 => 4, // size nibble 3 means a 4-byte payload
            n => n as usize,
        };
        let typ = (b >> 2) & 0x03;
        let tag = b >> 4;
        let data = &descriptor[i..i + size];
        i += size;
        match (typ, tag) {
            (1, 0x8) => report_id = data[0], // Report ID (global)
            (1, 0x0) => usage_page = if size == 2 { u16::from_le_bytes([data[0], data[1]]) } else { data[0] as u16 }, // Usage Page
            (2, 0x0) => usages.push(if size == 2 { u16::from_le_bytes([data[0], data[1]]) } else { data[0] as u16 }), // Usage (local)
            (2, 0x1) => usage_min = data[0] as u16, // Usage Minimum
            (2, 0x2) => usage_max = data[0] as u16, // Usage Maximum
            (1, 0x7) => report_size = data[0] as usize, // Report Size
            (1, 0x9) => report_count = data[0] as usize, // Report Count
            (0, 0xA) => {
                depth += 1; // Collection
                usages.clear();
            }
            (0, 0xC) => depth = depth.saturating_sub(1), // End Collection
            (0, 0x8) => {
                // Input
                let bits = report_size * report_count;
                if depth >= 1 {
                    out.input_bits += bits;
                    if data[0] & 0x01 == 0 {
                        // Data (not constant) fields inside the gamepad.
                        match usage_page {
                            0x09 => out.buttons = Some((usage_min, usage_max, report_size as u16)),
                            0x01 => {
                                for n in 0..report_count {
                                    let usage = *usages
                                        .get(n)
                                        .or_else(|| usages.last())
                                        .unwrap_or(&0);
                                    if usage == 0x39 {
                                        out.hat = Some(report_size as u16);
                                    } else {
                                        out.axes.push((usage, report_size as u16));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                usages.clear();
            }
            _ => {}
        }
    }
    out.report_id = report_id;
    out
}

fn app_owned_kit(kind: PadKind) -> PadKit {
    kit::factory()
        .kit(kind, ConnectionPath::AppOwned)
        .unwrap_or_else(|| panic!("no app-owned kit for {kind:?}"))
}

#[test]
fn authored_descriptor_lands_every_control_on_its_expected_kernel_code() {
    let kit = app_owned_kit(PadKind::DualSense);
    let w = walk(kit.descriptor());

    // Report id 1, byte-aligned input report.
    assert_eq!(w.report_id, 1);
    assert_eq!(w.input_bits % 8, 0);
    // 6 axes × 16 bits + 4-bit hat + 4 bits padding + 15 buttons + 1 pad bit.
    assert_eq!(w.input_bits, 6 * 16 + 4 + 4 + 15 + 1);

    // Six 16-bit axes in authored order: X, Y, Z, Rz, Rx, Ry.
    assert_eq!(
        w.axes,
        vec![
            (0x30, 16), // GD_X  → ABS_X
            (0x31, 16), // GD_Y  → ABS_Y
            (0x32, 16), // GD_Z  → ABS_Z   (L2)
            (0x35, 16), // GD_Rz → ABS_RZ  (R2)
            (0x33, 16), // GD_Rx → ABS_RX
            (0x34, 16), // GD_Ry → ABS_RY
        ]
    );
    // A 4-bit hat switch.
    assert_eq!(w.hat, Some(4));
    // Fifteen 1-bit buttons, ordinals 1..=15.
    assert_eq!(w.buttons, Some((1, 15, 1)));

    // The kit's declared kernel codes match the 4.4 contract exactly.
    let expected: &[(Button, u16)] = &[
        (Button::Cross, codes::BTN_SOUTH),
        (Button::Circle, codes::BTN_EAST),
        (Button::Triangle, codes::BTN_NORTH),
        (Button::Square, codes::BTN_WEST),
        (Button::L1, codes::BTN_TL),
        (Button::R1, codes::BTN_TR),
        (Button::Create, codes::BTN_SELECT),
        (Button::Options, codes::BTN_START),
        (Button::Guide, codes::BTN_MODE),
        (Button::L3, codes::BTN_THUMBL),
        (Button::R3, codes::BTN_THUMBR),
    ];
    for (button, code) in expected {
        assert_eq!(
            kit.kernel_code(*button),
            Some(*code),
            "button {button:?} lands on the wrong kernel code"
        );
    }
    // The spares land on the codes no SDL auto-mapping reads.
    assert_eq!(
        kit.spare_codes(),
        vec![codes::BTN_C, codes::BTN_Z, codes::BTN_TL2, codes::BTN_TR2]
    );
    // D-pad lands on hat 0.
    assert_eq!(kit.hat_codes(), (codes::ABS_HAT0X, codes::ABS_HAT0Y));
    // Sticks and triggers land on the axes SDL's auto-mapper reads.
    let axes = kit.axis_codes();
    assert_eq!(axes.left_stick, (codes::ABS_X, codes::ABS_Y));
    assert_eq!(axes.right_stick, (codes::ABS_RX, codes::ABS_RY));
    assert_eq!(axes.triggers, (codes::ABS_Z, codes::ABS_RZ));
}

#[test]
fn authored_report_encoding_matches_the_descriptor_layout() {
    // The authored report bytes must land every control in the byte position
    // the descriptor declares: report id, 6×16-bit axes, hat byte, then the
    // 2-byte button field.
    let kit = app_owned_kit(PadKind::DualSense);

    let report = PadInput {
        buttons: Button::Cross.bit()
            | Button::Square.bit()
            | Button::Options.bit()
            | Button::R3.bit(),
        hat: 2, // authored hat ordinal: 2 = right
        lx: 0x0000,
        ly: 0xFFFF,
        l2: 0x1234,
        r2: 0xABCD,
        rx: 0x8000,
        ry: 0x0200,
        battery: None,
    };
    let bytes = kit.encode_authored_report(&report);

    assert_eq!(bytes[0], 1, "report id");
    assert_eq!(&bytes[1..3], &0x0000u16.to_le_bytes(), "left stick X");
    assert_eq!(&bytes[3..5], &0xFFFFu16.to_le_bytes(), "left stick Y");
    assert_eq!(&bytes[5..7], &0x1234u16.to_le_bytes(), "L2 (Z)");
    assert_eq!(&bytes[7..9], &0xABCDu16.to_le_bytes(), "R2 (Rz)");
    assert_eq!(&bytes[9..11], &0x8000u16.to_le_bytes(), "right stick X (Rx)");
    assert_eq!(&bytes[11..13], &0x0200u16.to_le_bytes(), "right stick Y (Ry)");
    assert_eq!(bytes[13] & 0x0F, 2, "hat in low nibble of byte 13");
    let button_bits = u16::from_le_bytes([bytes[14], bytes[15]]);
    assert!(button_bits & Button::Cross.bit() != 0, "cross bit");
    assert!(button_bits & Button::Square.bit() != 0, "square bit");
    assert!(button_bits & Button::Options.bit() != 0, "options bit");
    assert!(button_bits & Button::R3.bit() != 0, "R3 bit");
    assert_eq!(button_bits & Button::Circle.bit(), 0, "circle released");
}

#[test]
fn every_app_owned_kind_shares_the_same_authored_layout() {
    // One authored descriptor serves all pad kinds (the per-kind difference is
    // the *parse*, not the delivery) — input-delivery research §4.3.
    let reference = app_owned_kit(PadKind::DualSense).descriptor().to_vec();
    for kind in [PadKind::Ds4, PadKind::XboxOneS] {
        assert_eq!(
            app_owned_kit(kind).descriptor(),
            reference.as_slice(),
            "{kind:?} must share the authored descriptor"
        );
    }
}

#[test]
fn kind_path_matrix_is_first_class() {
    // The kit factory models the kind × path compatibility matrix; an
    // unsupported combination is a first-class product, not a panic
    // (design-patterns research, maintainer ruling on Abstract Factory).
    let f = kit::factory();

    // DualSense (Supported): both paths.
    assert!(f.kit(PadKind::DualSense, ConnectionPath::AppOwned).is_some());
    assert!(f.kit(PadKind::DualSense, ConnectionPath::Stock).is_some());
    // DS4 (Best effort): both paths.
    assert!(f.kit(PadKind::Ds4, ConnectionPath::AppOwned).is_some());
    assert!(f.kit(PadKind::Ds4, ConnectionPath::Stock).is_some());
    // Xbox One S (Supported): app-owned only — the stock path is unproven for it.
    assert!(f.kit(PadKind::XboxOneS, ConnectionPath::AppOwned).is_some());
    assert!(f.kit(PadKind::XboxOneS, ConnectionPath::Stock).is_none());
    // Generic HID (no tier): stock only — there is no per-kind parse for it,
    // so the app-owned session has nothing to offer it in 0.1.0.
    assert!(f.kit(PadKind::GenericHid, ConnectionPath::AppOwned).is_none());
    assert!(f.kit(PadKind::GenericHid, ConnectionPath::Stock).is_some());
}

#[test]
fn stock_kits_are_degenerate_by_design() {
    // On the stock path LG's stack owns parsing: the kit carries no authored
    // descriptor and no parser — connect-only.
    let stock = kit::factory()
        .kit(PadKind::DualSense, ConnectionPath::Stock)
        .unwrap();
    assert!(stock.descriptor().is_empty());
    assert!(stock.parse_raw(&[0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
}
