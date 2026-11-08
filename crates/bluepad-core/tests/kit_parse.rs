//! Kit seam tests: raw pad report bytes → named control states.
//!
//! Ground truth: linux `hid-playstation.c` report structs and SDL
//! `SDL_hidapi_ps5.c` / `SDL_hidapi_xboxone.c` Bluetooth packet handlers
//! (button masks, field offsets, battery bytes — all verified against the
//! sources cited in docs/research/controller-settings.md §2).

use bluepad_core::kit::PadKitFactory;
use bluepad_core::kit::{self, Battery, Button};
use bluepad_core::{ConnectionPath, PadKind};

fn parse(kind: PadKind, raw: &[u8]) -> bluepad_core::kit::PadInput {
    let kit = kit::factory()
        .kit(kind, ConnectionPath::AppOwned)
        .unwrap_or_else(|| panic!("no app-owned kit for {kind:?}"));
    kit.parse_raw(raw)
        .unwrap_or_else(|| panic!("kit for {kind:?} failed to parse report {raw:02x?}"))
}

/// DualSense simple Bluetooth input report 0x01 (10 bytes):
/// [0]=0x01, [1]=LX, [2]=LY, [3]=RX, [4]=RY,
/// [5]=hat(low nibble) | square(0x10) cross(0x20) circle(0x40) triangle(0x80),
/// [6]=L1(0x01) R1(0x02) L2(0x04) R2(0x08) create(0x10) options(0x20) L3(0x40) R3(0x80),
/// [7]=PS(0x01) touchpad(0x02), [8]=L2 analog, [9]=R2 analog.
fn dualsense_simple(buttons0: u8, buttons1: u8, buttons2: u8) -> Vec<u8> {
    vec![0x01, 0x80, 0x80, 0x80, 0x80, buttons0, buttons1, buttons2, 0x00, 0xFF]
}

#[test]
fn dualsense_simple_report_parses_every_control() {
    let input = parse(
        PadKind::DualSense,
        &dualsense_simple(0x20 | 0x80 | 0x02, 0x01 | 0x10 | 0x80, 0x01),
        // cross + triangle + dpad-right; L1 + create + R3; PS
    );
    assert!(input.pressed(Button::Cross));
    assert!(input.pressed(Button::Triangle));
    assert!(input.pressed(Button::L1));
    assert!(input.pressed(Button::Create));
    assert!(input.pressed(Button::R3));
    assert!(input.pressed(Button::Guide));
    assert!(!input.pressed(Button::Circle));
    assert!(!input.pressed(Button::Square));
    assert!(!input.pressed(Button::Options));
    assert_eq!(input.hat, 2, "dpad right (hat ordinal 2)");
    // Sticks centered, R2 fully pressed.
    assert_eq!(input.lx, 0x8080u16, "LX 0x80 scales to 0x8080 (v*257)");
    assert_eq!(input.r2, 0xFFFFu16, "R2 0xFF scales to full");
    assert_eq!(input.l2, 0x0, "L2 released");
    // No battery in the simple report.
    assert_eq!(input.battery, None);
}

#[test]
fn dualsense_enhanced_report_parses_controls_and_battery() {
    // DualSense enhanced Bluetooth input report 0x31 (78 bytes):
    // [0]=0x31, [1]=seq, [2]=LX, [3]=LY, [4]=RX, [5]=RY, [6]=L2, [7]=R2,
    // [8]=counter, [9]=buttons0, [10]=buttons1, [11]=buttons2,
    // battery capacity = low nibble of byte 54 (0-10).
    let mut raw = vec![0u8; 78];
    raw[0] = 0x31;
    raw[2] = 0x00; // LX
    raw[3] = 0xFF; // LY
    raw[9] = 0x40 | 0x04; // circle + dpad-down (hat ordinal 4)
    raw[10] = 0x02 | 0x20 | 0x40; // R1 + options + L3
    raw[11] = 0x02; // touchpad (not a standard control; ignored)
    raw[6] = 0xFF; // L2
    raw[54] = 0x0A; // battery 10/10
    let input = parse(PadKind::DualSense, &raw);
    assert!(input.pressed(Button::Circle));
    assert!(input.pressed(Button::R1));
    assert!(input.pressed(Button::Options));
    assert!(input.pressed(Button::L3));
    assert_eq!(input.hat, 4, "dpad down");
    assert_eq!(input.lx, 0x0);
    assert_eq!(input.ly, 0xFFFF);
    assert_eq!(input.l2, 0xFFFF);
    assert_eq!(input.r2, 0x0);
    assert_eq!(
        input.battery,
        Some(Battery {
            percent: 100,
            charging: false
        })
    );
}

/// DS4 Bluetooth input report 0x11 (78 bytes): [0]=0x11, [1..2] reserved,
/// common at [3]: x, y, rx, ry, buttons[3], z, rz …, status[0] at [33]
/// (battery capacity low nibble, 0-10, 11 = full).
#[test]
fn ds4_bt_report_parses_controls_and_battery() {
    let mut raw = vec![0u8; 78];
    raw[0] = 0x11;
    raw[3] = 0x80; // x
    raw[4] = 0x7F; // y
    raw[5] = 0x40; // rx
    raw[6] = 0xC0; // ry
    raw[7] = 0x10 | 0x20; // square + cross, hat 0 = dpad-up
    raw[8] = 0x04 | 0x08; // L2 + R2 digital
    raw[9] = 0x01; // PS
    raw[10] = 0x7F; // z (L2 analog)
    raw[11] = 0x00; // rz
    raw[33] = 0x0B; // battery: full (11)
    let input = parse(PadKind::Ds4, &raw);
    assert!(input.pressed(Button::Square));
    assert!(input.pressed(Button::Cross));
    assert!(input.pressed(Button::Guide));
    assert_eq!(input.hat, 0, "dpad up");
    assert_eq!(input.l2, 0x7F * 257, "L2 analog scaled to 16 bits");
    assert_eq!(input.r2, 0x0);
    assert_eq!(
        input.battery,
        Some(Battery {
            percent: 100,
            charging: false
        })
    );
}

/// DS4 simple Bluetooth input report 0x01 (10 bytes):
/// [0]=0x01, [1]=x, [2]=y, [3]=rx, [4]=ry, [5]=buttons0, [6]=buttons1,
/// [7]=buttons2, [8]=z, [9]=rz.
#[test]
fn ds4_simple_report_parses_controls() {
    let raw = vec![
        0x01, 0x80, 0x80, 0x80, 0x80, 0x80 | 0x06, 0x01 | 0x20, 0x00, 0x00, 0xFF,
    ];
    let input = parse(PadKind::Ds4, &raw);
    assert!(input.pressed(Button::Triangle)); // bit 7 of buttons0
    assert!(input.pressed(Button::L1));
    assert!(input.pressed(Button::Options));
    assert_eq!(input.hat, 6, "dpad left");
    assert_eq!(input.r2, 0xFFFF);
    assert_eq!(input.battery, None);
}

/// Xbox One S Bluetooth input report 0x01 (17 bytes, fw 5.x):
/// [0]=0x01, [1..3]=LX LE16, [3..5]=LY, [5..7]=RX, [7..9]=RY (centered 0x8000),
/// [9..11]=LT LE16 (0-1023), [11..13]=RT, [13]=hat (1=up … 8=upleft, else neutral),
/// [14]=A(0x01) B(0x02) X(0x08) Y(0x10) LB(0x40) RB(0x80),
/// [15]=back(0x04) start(0x08) guide(0x10) L3(0x20) R3(0x40).
#[test]
fn xbox_one_s_bt_report_parses_controls() {
    let mut raw = vec![0u8; 17];
    raw[0] = 0x01;
    raw[1..3].copy_from_slice(&0x8000u16.to_le_bytes());
    raw[3..5].copy_from_slice(&0x1000u16.to_le_bytes());
    raw[5..7].copy_from_slice(&0x8000u16.to_le_bytes());
    raw[7..9].copy_from_slice(&0xF000u16.to_le_bytes());
    raw[9..11].copy_from_slice(&1023u16.to_le_bytes());
    raw[11..13].copy_from_slice(&512u16.to_le_bytes());
    raw[13] = 5; // dpad down
    raw[14] = 0x01 | 0x08 | 0x40; // A + X + LB
    raw[15] = 0x04 | 0x10 | 0x40; // back + guide + R3
    let input = parse(PadKind::XboxOneS, &raw);
    // Xbox glyphs map onto the standard layout: A→cross, B→circle, X→square, Y→triangle.
    assert!(input.pressed(Button::Cross));
    assert!(input.pressed(Button::Square));
    assert!(input.pressed(Button::L1));
    assert!(input.pressed(Button::Create));
    assert!(input.pressed(Button::Guide));
    assert!(input.pressed(Button::R3));
    assert!(!input.pressed(Button::Triangle));
    assert_eq!(input.hat, 4, "dpad down (Xbox hat 5 → authored ordinal 4)");
    assert_eq!(input.lx, 0x8000, "centered stick passes through");
    assert_eq!(input.ly, 0x1000);
    // Triggers: 0-1023 scaled to 0-65535.
    assert_eq!(input.l2, 65535, "LT 1023 scales to full");
    assert_eq!(input.r2, 512 * 64, "RT 512 scales to 32768-ish");
}

/// Xbox battery report 0x04: [1] = flags; capacity bits 0-1 → 10/40/70/100 %,
/// bits 2-3 zero → on USB (charging).
#[test]
fn xbox_battery_report_parses() {
    let mut raw = vec![0u8; 4];
    raw[0] = 0x04;
    raw[1] = 0x06; // 70 %, bits 2-3 set → on battery
    let input = parse(PadKind::XboxOneS, &raw);
    assert_eq!(
        input.battery,
        Some(Battery {
            percent: 70,
            charging: false
        })
    );
}

#[test]
fn unknown_reports_parse_to_none() {
    let kit = kit::factory().kit(PadKind::DualSense, ConnectionPath::AppOwned).unwrap();
    assert!(kit.parse_raw(&[]).is_none());
    assert!(kit.parse_raw(&[0x55, 1, 2, 3]).is_none(), "unknown report id");
    assert!(
        kit.parse_raw(&[0x01, 1]).is_none(),
        "truncated simple report"
    );
}

#[test]
fn diagnostic_readings_pair_interpreted_labels_with_raw_codes() {
    // The pad diagnostic renders every control with its interpreted label and
    // raw code side by side (spec: Pad diagnostic).
    let kit = kit::factory().kit(PadKind::DualSense, ConnectionPath::AppOwned).unwrap();
    let input = parse(
        PadKind::DualSense,
        &dualsense_simple(0x20, 0x00, 0x00), // cross pressed, hat neutral
    );
    let readings = kit.diagnostic_readings(&input);
    let cross = readings
        .iter()
        .find(|r| r.label == "Cross")
        .expect("cross reading present");
    assert_eq!(cross.code, Some(0x130), "BTN_SOUTH");
    assert_eq!(cross.value, 1);
    let dpad_right = readings
        .iter()
        .find(|r| r.label == "D-pad right")
        .expect("dpad reading present");
    assert_eq!(dpad_right.code, Some(0x10), "ABS_HAT0X");
    // Raw-only controls (unknown to the per-kind parse) still appear.
    let lx = readings
        .iter()
        .find(|r| r.label == "Left stick X")
        .expect("stick reading present");
    assert_eq!(lx.code, Some(0x00), "ABS_X");
}
