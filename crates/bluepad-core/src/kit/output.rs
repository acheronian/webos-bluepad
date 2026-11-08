//! Output report encoders: settings → pad-side output reports.
//!
//! Layouts from linux `hid-playstation.c` (`dualsense_output_report_bt`,
//! `dualshock4_output_report_bt`) and SDL `SDL_hidapi_xboxone.c` (Bluetooth
//! rumble packet). Output features ride ordinary HID output reports on the
//! app-owned session — no kernel driver needed (controller-settings
//! research §2).

use super::crc32::ps_crc;
use super::{Lightbar, OutputRequest, PadKind, PlayerIndex};

pub type OutputEncoder = fn(&OutputRequest, u8) -> Vec<u8>;

pub fn encoder_for(kind: PadKind) -> OutputEncoder {
    match kind {
        PadKind::DualSense => encode_dualsense,
        PadKind::Ds4 => encode_ds4,
        PadKind::XboxOneS => encode_xbox,
        PadKind::GenericHid => encode_none,
    }
}

fn encode_none(_request: &OutputRequest, _seq: u8) -> Vec<u8> {
    vec![]
}

/// DualSense Bluetooth output report 0x31 (78 bytes). Offsets per
/// `dualsense_output_report_bt`: id 0, seq_tag 1, tag 2, common at 3 —
/// valid_flag0 3, valid_flag1 4, motor_right 5, motor_left 6, …,
/// valid_flag2 41, lightbar_setup 44, led_brightness 45, player_leds 46,
/// lightbar RGB 47-49 — reserved to 73, CRC32 LE at 74-77.
fn encode_dualsense(request: &OutputRequest, seq: u8) -> Vec<u8> {
    let mut r = vec![0u8; 78];
    r[0] = 0x31; // report id
    r[1] = seq << 4; // sequence number, high nibble
    r[2] = 0x10; // BT tag

    if let Some(rumble) = request.rumble {
        // Classic rumble: haptics select + compatible vibration
        // (hid-playstation.c dualsense_output_worker).
        r[3] |= 0x03; // valid_flag0: COMPATIBLE_VIBRATION | HAPTICS_SELECT
        r[5] = rumble.high; // motor_right
        r[6] = rumble.low; // motor_left
    }
    if request.lightbar.is_some() || request.player_leds.is_some() {
        if request.lightbar.is_some() {
            r[4] |= 0x04; // valid_flag1: LIGHTBAR_CONTROL_ENABLE
            r[41] |= 0x02; // valid_flag2: LIGHTBAR_SETUP_CONTROL_ENABLE
            r[44] = 0x02; // lightbar_setup: LIGHT_OUT
        }
        if let Some(leds) = request.player_leds {
            r[4] |= 0x10; // valid_flag1: PLAYER_INDICATOR_CONTROL_ENABLE
            r[46] = leds;
        }
        if let Some(lb) = request.lightbar {
            r[47] = lb.red;
            r[48] = lb.green;
            r[49] = lb.blue;
        }
    }
    let crc = ps_crc(0xA2, &r[0..74]);
    r[74..78].copy_from_slice(&crc.to_le_bytes());
    r
}

/// DualShock 4 Bluetooth output report 0x11 (78 bytes). Offsets per
/// `dualshock4_output_report_bt`: id 0, hw_control 1, audio_control 2,
/// common at 3 — valid_flag0 3, valid_flag1 4, reserved 5, motor_right 6,
/// motor_left 7, lightbar RGB 8-10, blink 11-12 — CRC32 LE at 74-77.
///
/// Rumble and lightbar are always sent together (`valid_flag0 >= 0x03`,
/// third-party pad compatibility — hid-playstation.c).
fn encode_ds4(request: &OutputRequest, _seq: u8) -> Vec<u8> {
    let mut r = vec![0u8; 78];
    r[0] = 0x11; // report id
    r[1] = 0xC0; // hw_control: HID command (0x80) | CRC present (0x40)

    let (rumble, lightbar) = (request.rumble, request.lightbar);
    if rumble.is_some() || lightbar.is_some() {
        r[3] |= 0x03; // valid_flag0: MOTOR | LED, always together
        if let Some(ru) = rumble {
            r[6] = ru.high; // motor_right
            r[7] = ru.low; // motor_left
        }
        if let Some(lb) = lightbar {
            r[8] = lb.red;
            r[9] = lb.green;
            r[10] = lb.blue;
        }
    }
    let crc = ps_crc(0xA2, &r[0..74]);
    r[74..78].copy_from_slice(&crc.to_le_bytes());
    r
}

/// Xbox One S Bluetooth rumble report 0x03 (9 bytes):
/// { 0x03, 0x0F, LT, RT, low, high, 0xFF, 0x00, 0xEB }
/// (SDL_hidapi_xboxone.c, `HIDAPI_DriverXboxOne_UpdateRumble`).
/// LED requests are dropped: the One S has no LED output report over
/// Bluetooth.
fn encode_xbox(request: &OutputRequest, _seq: u8) -> Vec<u8> {
    let rumble = request.rumble.unwrap_or_default();
    vec![
        0x03, 0x0F, rumble.impulse_left, rumble.impulse_right, rumble.low, rumble.high, 0xFF,
        0x00, 0xEB,
    ]
}

/// DualSense player-LED masks: PS5 centered patterns
/// (player 1 `--x--`, 2 `-xx--`, 3 `x-x-x`, 4 `xxxxx`).
pub fn dualsense_player_led_mask(player: PlayerIndex) -> u8 {
    match player {
        PlayerIndex::P1 => 0x04,
        PlayerIndex::P2 => 0x06,
        PlayerIndex::P3 => 0x15,
        PlayerIndex::P4 => 0x1F,
    }
}

/// PS4 lightbar player colors (hid-playstation.c,
/// `dualshock4_set_default_lightbar_colors`).
pub fn player_lightbar_color(player: PlayerIndex) -> Lightbar {
    match player {
        PlayerIndex::P1 => Lightbar {
            red: 0x00,
            green: 0x00,
            blue: 0x40,
        },
        PlayerIndex::P2 => Lightbar {
            red: 0x40,
            green: 0x00,
            blue: 0x00,
        },
        PlayerIndex::P3 => Lightbar {
            red: 0x00,
            green: 0x40,
            blue: 0x00,
        },
        PlayerIndex::P4 => Lightbar {
            red: 0x20,
            green: 0x00,
            blue: 0x20,
        },
    }
}
