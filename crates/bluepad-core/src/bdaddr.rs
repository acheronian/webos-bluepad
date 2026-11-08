//! Bluetooth device address newtype.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A Bluetooth device address (BD_ADDR), the stable key for per-pad state.
///
/// Normalized form is lowercase hex without separators (`aabbccddeeff`) — the
/// settings and registry keying (controller-settings research §4.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BdAddr(#[serde(with = "serde_bd_addr")] [u8; 6]);

impl BdAddr {
    pub fn new(bytes: [u8; 6]) -> Self {
        BdAddr(bytes)
    }

    pub fn bytes(&self) -> [u8; 6] {
        self.0
    }

    /// The normalized registry/settings key: lowercase hex, no separators.
    pub fn key(&self) -> String {
        self.to_string()
    }

    /// The LS2 wire format: colon-separated lowercase hex
    /// (`aa:bb:cc:dd:ee:ff`) — the exact string bluetooth2 reports in
    /// `device/getStatus` and expects in outgoing payloads. Sending the
    /// normalized key instead makes the stack ACK the call but never
    /// match the device (captured: phantom `e8473ab85a2f` entry +
    /// pair subscriptions that never reach `endPairing`).
    pub fn colon(&self) -> String {
        let [a, b, c, d, e, f] = self.0;
        format!("{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}")
    }
}

impl fmt::Display for BdAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for BdAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BdAddr({self})")
    }
}

impl FromStr for BdAddr {
    type Err = BdAddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: String = s
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if hex.len() != 12 {
            return Err(BdAddrParseError);
        }
        let mut bytes = [0u8; 6];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16).unwrap() as u8;
            let lo = (chunk[1] as char).to_digit(16).unwrap() as u8;
            bytes[i] = (hi << 4) | lo;
        }
        Ok(BdAddr(bytes))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("not a Bluetooth address")]
pub struct BdAddrParseError;

mod serde_bd_addr {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
        s.serialize_str(&hex)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 6], D::Error> {
        let s = String::deserialize(d)?;
        let mut out = [0u8; 6];
        let b = s.as_bytes();
        if b.len() == 12 {
            for i in 0..6 {
                let hi = (b[2 * i] as char).to_digit(16).ok_or_else(|| {
                    serde::de::Error::custom("invalid hex in Bluetooth address")
                })?;
                let lo = (b[2 * i + 1] as char).to_digit(16).ok_or_else(|| {
                    serde::de::Error::custom("invalid hex in Bluetooth address")
                })?;
                out[i] = ((hi as u8) << 4) | lo as u8;
            }
            return Ok(out);
        }
        Err(serde::de::Error::custom("Bluetooth address must be 12 hex digits"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_common_address_formats() {
        for s in ["a0:fa:9c:62:7d:1a", "A0FA9C627D1A", "a0-fa-9c-62-7d-1a"] {
            let a: BdAddr = s.parse().unwrap();
            assert_eq!(a.key(), "a0fa9c627d1a");
            assert_eq!(a.bytes(), [0xa0, 0xfa, 0x9c, 0x62, 0x7d, 0x1a]);
        }
        assert!("a0:fa:9c:62:7d".parse::<BdAddr>().is_err());
        assert!("zz".parse::<BdAddr>().is_err());
    }

    #[test]
    fn wire_format_is_colon_separated() {
        let a: BdAddr = "a0fa9c627d1a".parse().unwrap();
        assert_eq!(a.colon(), "a0:fa:9c:62:7d:1a");
    }

    #[test]
    fn serde_round_trips_through_the_normalized_key() {
        let a: BdAddr = "A0:FA:9C:62:7D:1A".parse().unwrap();
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, "\"a0fa9c627d1a\"");
        let back: BdAddr = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }
}
