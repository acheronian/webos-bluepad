//! CRC-32 (IEEE 802.3, reflected, poly 0xEDB88320) — the algorithm behind
//! the kernel's `crc32_le`, used to sign Bluetooth HID reports.

/// Standard CRC-32 (IEEE 802.3): reflected register from 0xFFFFFFFF with
/// final inversion — the value `crc32_le(0xFFFFFFFF, data)` yields.
pub fn crc32(data: &[u8]) -> u32 {
    !crc32_running(0xFFFF_FFFF, data)
}

fn crc32_running(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    crc
}

/// The PlayStation Bluetooth report signature (hid-playstation.c,
/// `dualsense_send_output_report`):
///
/// ```c
/// crc = crc32_le(0xFFFFFFFF, &seed, 1);
/// crc = ~crc32_le(crc, report->data, report->len - 4);
/// ```
///
/// `crc32_le` is the raw reflected register update (v4.4 lib/crc32.c), so
/// the composed expression is `~body(0xFFFFFFFF, seed ++ payload)` — one
/// register walk over the seed byte and the report, inverted at the end.
/// Verified to agree with SDL's field-proven scheme
/// (`SDL_crc32(SDL_crc32(0, [0xA2]), payload)`).
pub fn ps_crc(seed: u8, payload: &[u8]) -> u32 {
    !crc32_running(crc32_running(0xFFFF_FFFF, &[seed]), payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn ps_crc_seeds_and_inverts_like_the_kernel() {
        // Independent recomputation with one running register over
        // seed-then-payload, then inversion.
        let payload = [0x31u8, 0x00, 0x10, 0x03];
        let mut reg = 0xFFFF_FFFFu32;
        for &b in [0xA2u8].iter().chain(payload.iter()) {
            reg ^= b as u32;
            for _ in 0..8 {
                reg = if reg & 1 != 0 { (reg >> 1) ^ 0xEDB8_8320 } else { reg >> 1 };
            }
        }
        assert_eq!(ps_crc(0xA2, &payload), !reg);
    }
}
