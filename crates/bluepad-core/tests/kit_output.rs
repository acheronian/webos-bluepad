//! Kit seam tests: settings → pad output reports.
//!
//! Ground truth: linux `hid-playstation.c` DualSense/DualShock4 Bluetooth
//! output report layouts and CRC scheme, SDL `SDL_hidapi_xboxone.c` Bluetooth
//! rumble packet, and the PS player-indicator conventions documented in
//! docs/research/controller-settings.md §1–§2.

use bluepad_core::kit::PadKitFactory;
use bluepad_core::kit::{self, Lightbar, OutputRequest, PlayerIndex, Rumble};
use bluepad_core::{ConnectionPath, PadKind};

fn kit_for(kind: PadKind) -> bluepad_core::kit::PadKit {
    kit::factory()
        .kit(kind, ConnectionPath::AppOwned)
        .unwrap_or_else(|| panic!("no app-owned kit for {kind:?}"))
}

/// Independent CRC-32 (IEEE 802.3, reflected, poly 0xEDB88320) — the same
/// algorithm the kernel's crc32_le implements, recomputed here from the
/// documented scheme so the kit cannot hide a bug behind shared code.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[test]
fn crc32_known_vector() {
    // Standard check value for "123456789".
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn dualsense_output_report_carries_rumble_lightbar_and_player_leds() {
    let kit = kit_for(PadKind::DualSense);
    let report = kit
        .build_output(
            &OutputRequest {
                rumble: Some(Rumble {
                    low: 0x80,
                    high: 0x40,
                    impulse_left: 0,
                    impulse_right: 0,
                }),
                lightbar: Some(Lightbar {
                    red: 0x12,
                    green: 0x34,
                    blue: 0x56,
                }),
                player_leds: Some(0x04),
            },
            3, // sequence number
        )
        .expect("DualSense kit builds output reports");

    assert_eq!(report.len(), 78);
    assert_eq!(report[0], 0x31, "report id");
    assert_eq!(report[1], 0x30, "sequence number in high nibble");
    assert_eq!(report[2], 0x10, "BT tag");
    // Common section starts at byte 3.
    assert_eq!(report[3], 0x03, "valid_flag0: vibration | haptics select");
    assert_eq!(report[4], 0x14, "valid_flag1: lightbar | player indicator");
    assert_eq!(report[5], 0x40, "motor_right = high-frequency");
    assert_eq!(report[6], 0x80, "motor_left = low-frequency");
    assert_eq!(report[41], 0x02, "valid_flag2: lightbar setup");
    assert_eq!(report[44], 0x02, "lightbar_setup = light out");
    assert_eq!(report[46], 0x04, "player LEDs mask");
    assert_eq!(report[47], 0x12, "lightbar red");
    assert_eq!(report[48], 0x34, "lightbar green");
    assert_eq!(report[49], 0x56, "lightbar blue");
    // CRC: ~crc32(crc32(0xFFFFFFFF, [0xA2]), report[0..74])
    let expected = crc32_seed_0xa2(&report[0..74]);
    assert_eq!(
        u32::from_le_bytes([report[74], report[75], report[76], report[77]]),
        expected,
        "trailing CRC32"
    );
}

fn crc32_seed_0xa2(payload: &[u8]) -> u32 {
    // crc = crc32_le(0xFFFFFFFF, [0xA2]); crc = ~crc32_le(crc, payload)
    let mut crc = 0xFFFF_FFFFu32;
    for &b in &[0xA2u8] {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    for &b in payload {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[test]
fn ds4_output_report_carries_rumble_and_lightbar() {
    let kit = kit_for(PadKind::Ds4);
    let report = kit
        .build_output(
            &OutputRequest {
                rumble: Some(Rumble {
                    low: 0xFF,
                    high: 0x00,
                    impulse_left: 0,
                    impulse_right: 0,
                }),
                lightbar: Some(Lightbar {
                    red: 0x40,
                    green: 0x00,
                    blue: 0x00,
                }),
                player_leds: None,
            },
            0,
        )
        .expect("DS4 kit builds output reports");

    assert_eq!(report.len(), 78);
    assert_eq!(report[0], 0x11, "report id");
    assert_eq!(report[1], 0xC0, "hw_control: HID command | CRC present");
    // Common section starts at byte 3.
    assert_eq!(report[3], 0x03, "valid_flag0: motor | LED (always together)");
    assert_eq!(report[6], 0x00, "motor_right = high-frequency");
    assert_eq!(report[7], 0xFF, "motor_left = low-frequency");
    assert_eq!(report[8], 0x40, "lightbar red");
    assert_eq!(report[9], 0x00, "lightbar green");
    assert_eq!(report[10], 0x00, "lightbar blue");
    let expected = crc32_seed_0xa2(&report[0..74]);
    assert_eq!(
        u32::from_le_bytes([report[74], report[75], report[76], report[77]]),
        expected,
        "trailing CRC32"
    );
}

#[test]
fn xbox_output_report_is_the_bluetooth_rumble_packet() {
    let kit = kit_for(PadKind::XboxOneS);
    let report = kit
        .build_output(
            &OutputRequest {
                rumble: Some(Rumble {
                    low: 0x40,
                    high: 0x80,
                    impulse_left: 0x10,
                    impulse_right: 0x20,
                }),
                lightbar: None,
                player_leds: None,
            },
            0,
        )
        .expect("Xbox kit builds rumble reports");

    // SDL_hidapi_xboxone.c Bluetooth rumble packet:
    // { 0x03, 0x0F, LT, RT, low, high, 0xFF, 0x00, 0xEB }
    assert_eq!(
        report,
        vec![0x03, 0x0F, 0x10, 0x20, 0x40, 0x80, 0xFF, 0x00, 0xEB]
    );
}

#[test]
fn xbox_kit_ignores_led_requests_it_cannot_honor() {
    // The One S has no LED output report over Bluetooth (controller-settings
    // research §2.3): LED requests are dropped, not fabricated.
    let kit = kit_for(PadKind::XboxOneS);
    let report = kit
        .build_output(
            &OutputRequest {
                rumble: None,
                lightbar: Some(Lightbar {
                    red: 1,
                    green: 2,
                    blue: 3,
                }),
                player_leds: Some(0x1F),
            },
            0,
        )
        .expect("rumble-less LED-only request still yields the idle packet");
    assert_eq!(report, vec![0x03, 0x0F, 0, 0, 0, 0, 0xFF, 0x00, 0xEB]);
}

#[test]
fn player_indicators_follow_the_console_conventions() {
    // DualSense player LEDs: PS5 centered patterns (hid-playstation.c /
    // controller-settings research §1.1).
    assert_eq!(
        kit::dualsense_player_led_mask(PlayerIndex::P1),
        0x04,
        "player 1: --x--"
    );
    assert_eq!(
        kit::dualsense_player_led_mask(PlayerIndex::P2),
        0x06,
        "player 2: -xx--"
    );
    assert_eq!(
        kit::dualsense_player_led_mask(PlayerIndex::P3),
        0x15,
        "player 3: x-x-x"
    );
    assert_eq!(
        kit::dualsense_player_led_mask(PlayerIndex::P4),
        0x1F,
        "player 4: xxxxx"
    );
    // PS4 lightbar player colors (hid-playstation.c,
    // dualshock4_set_default_lightbar_colors).
    assert_eq!(
        kit::player_lightbar_color(PlayerIndex::P1),
        Lightbar {
            red: 0x00,
            green: 0x00,
            blue: 0x40
        },
        "player 1: blue"
    );
    assert_eq!(
        kit::player_lightbar_color(PlayerIndex::P4),
        Lightbar {
            red: 0x20,
            green: 0x00,
            blue: 0x20
        },
        "player 4: pink"
    );
}
