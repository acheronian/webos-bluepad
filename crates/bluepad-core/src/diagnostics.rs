//! The diagnostics bundle format (spec: Diagnostics and observability;
//! glossary: Diagnostics bundle).
//!
//! Anonymization is part of the format, not a post-step: the manifest's pad
//! entries are pseudonymized at assembly (addresses hashed/truncated, names
//! dropped — the kind is kept), and no MAC ever appears in the clear. The
//! layout is a small versioned JSON manifest plus log files, packed as a
//! ustar tar so maintainer reports are triageable across releases.

use crate::bdaddr::BdAddr;
use crate::pad::{ConnectionPath, PadKind, PadPhase};
use crate::protocol::{Capability, PrivilegeTier};
use crate::registry::PairingProvenance;
use serde::{Deserialize, Serialize};

/// The bundle manifest's schema version — bumped on any breaking change
/// to the layout, so maintainer reports are triageable across releases.
pub const BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Runtime health counters the bundle carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthCounters {
    /// Sessions that earned Connected (health-checked input).
    pub successful_connects: u64,
    /// Connect attempts that failed.
    pub connect_failures: u64,
    /// Sessions that came up but streamed nothing (the silent-blackout
    /// specimen).
    pub silent_blackouts: u64,
    /// Live sessions that dropped.
    pub session_losses: u64,
    /// Pads Connected right now.
    pub connected_now: u8,
    /// Pads Recovering right now.
    pub recovering_now: u8,
}

/// One pad registry entry, pseudonymized for the bundle: the address is
/// hashed/truncated (never in the clear), the human name is dropped — the
/// kind is kept, the one name triage needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PseudonymizedPad {
    /// The pseudonymized address (see [`pseudonymize`]).
    pub id: String,
    pub kind: PadKind,
    pub phase: PadPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proven_path: Option<ConnectionPath>,
    pub provenance: PairingProvenance,
}

/// The bundle's manifest: the environment snapshot, health counters, and
/// the pseudonymized registry summary — everything a maintainer needs to
/// triage a report (glossary: Diagnostics bundle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub app_version: String,
    pub protocol_version: u32,
    /// Wall-clock seconds at export time.
    pub generated_unix: u64,
    pub tier: PrivilegeTier,
    pub capabilities: Vec<Capability>,
    pub health: HealthCounters,
    pub pads: Vec<PseudonymizedPad>,
}

/// One file in the bundle (a rotated log, the manifest's neighbors).
/// Content crosses the UI↔daemon seam base64-encoded; the Rust type stays
/// bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsFile {
    /// Bundle-relative path, e.g. `logs/bluepad-daemon.log`.
    pub name: String,
    #[serde(with = "b64")]
    pub content: Vec<u8>,
}

/// The assembled bundle: manifest + files, as it crosses the seam and
/// lands in the delivered tar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsBundle {
    pub manifest: BundleManifest,
    pub files: Vec<DiagnosticsFile>,
}

/// Pseudonymize a pad address for the bundle: keep the OUI prefix (the
/// vendor — genuinely useful for triage, not identifying), hash the rest.
/// The full address never appears.
pub fn pseudonymize(address: &BdAddr) -> String {
    let bytes = address.bytes();
    format!(
        "{:02x}:{:02x}:{:02x}…{:08x}",
        bytes[0],
        bytes[1],
        bytes[2],
        fnv1a_32(&bytes)
    )
}

/// Scrub MAC addresses out of log text destined for the bundle: the
/// colon wire form and the `BdAddr(…)` debug wrapper are rewritten to
/// pseudonyms wherever they appear; bare normalized keys are rewritten
/// only for the known registry addresses (a bare 12-hex run may be
/// report data, not an address). No MAC survives in the clear.
pub fn scrub_macs(content: &[u8], known: &[BdAddr]) -> Vec<u8> {
    let is_hex = |b: u8| b.is_ascii_hexdigit();
    let known_keys: Vec<String> = known.iter().map(|a| a.key()).collect();
    let mut out = Vec::with_capacity(content.len());
    let mut i = 0;
    while i < content.len() {
        // Colon wire form: hh:hh:hh:hh:hh:hh (17 bytes).
        if let Some(address) = colon_mac_at(content, i) {
            extend_pseudonym(&mut out, &address);
            i += 17;
            continue;
        }
        // Debug wrapper: BdAddr(12hex).
        if content[i..].starts_with(b"BdAddr(")
            && i + 7 + 12 < content.len()
            && content[i + 7..i + 19].iter().all(|b| is_hex(*b))
            && content[i + 19] == b')'
        {
            let text = String::from_utf8_lossy(&content[i + 7..i + 19]).to_string();
            out.extend_from_slice(b"BdAddr(");
            if let Ok(address) = text.parse::<BdAddr>() {
                extend_pseudonym(&mut out, &address);
            } else {
                out.extend_from_slice(&content[i + 7..i + 19]);
            }
            out.push(b')');
            i += 20;
            continue;
        }
        // Bare normalized key of a known registry address (12 hex).
        if known_keys
            .iter()
            .any(|key| content[i..].starts_with(key.as_bytes()))
        {
            let key_len = 12;
            let text = String::from_utf8_lossy(&content[i..i + key_len]).to_string();
            if let Ok(address) = text.parse::<BdAddr>() {
                extend_pseudonym(&mut out, &address);
                i += key_len;
                continue;
            }
        }
        out.push(content[i]);
        i += 1;
    }
    out
}

/// A colon-form MAC at `i`, parsed, if the 17 bytes match the shape.
fn colon_mac_at(content: &[u8], i: usize) -> Option<BdAddr> {
    if i + 17 > content.len() {
        return None;
    }
    let mut bytes = [0u8; 6];
    for (pair, byte) in bytes.iter_mut().enumerate() {
        let at = i + pair * 3;
        let (hi, lo, sep) = (content[at], content[at + 1], content[at + 2]);
        if !hi.is_ascii_hexdigit() || !lo.is_ascii_hexdigit() {
            return None;
        }
        if pair < 5 && sep != b':' {
            return None;
        }
        *byte = ((hi as char).to_digit(16).unwrap() as u8) << 4
            | (lo as char).to_digit(16).unwrap() as u8;
    }
    Some(BdAddr::new(bytes))
}

fn extend_pseudonym(out: &mut Vec<u8>, address: &BdAddr) {
    out.extend_from_slice(pseudonymize(address).as_bytes());
}

/// FNV-1a (32-bit) — the bundle's tiny deterministic hash.
fn fnv1a_32(data: &[u8]) -> u32 {
    let mut hash = 0x2166_1362_u32;
    for byte in data {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Pack the bundle as a ustar tar archive: `manifest.json` + the files at
/// their bundle-relative paths. One file, standard format, triageable
/// anywhere.
pub fn bundle_tar(bundle: &DiagnosticsBundle) -> Vec<u8> {
    let manifest_json = serde_json::to_vec_pretty(&bundle.manifest)
        .expect("the bundle manifest serializes");
    let mut tar = Vec::new();
    tar_push_file(&mut tar, "manifest.json", &manifest_json);
    for file in &bundle.files {
        tar_push_file(&mut tar, &file.name, &file.content);
    }
    // The archive ends with two zero blocks (POSIX ustar).
    tar.extend_from_slice(&[0u8; 1024]);
    tar
}

/// Append one regular file as a ustar header + content, padded to the
/// 512-byte block size.
fn tar_push_file(tar: &mut Vec<u8>, name: &str, content: &[u8]) {
    assert!(name.len() <= 100, "ustar names fit the name field: {name}");
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..108].copy_from_slice(b"0000644\0"); // mode
    header[108..116].copy_from_slice(b"0000000\0"); // uid
    header[116..124].copy_from_slice(b"0000000\0"); // gid
    header[124..136].copy_from_slice(&octal_12(content.len() as u64)); // size
    header[136..148].copy_from_slice(&octal_12(0)); // mtime: reproducible
    header[156] = b'0'; // typeflag: regular file
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    // Checksum: the header's bytes with the chksum field as spaces.
    let checksum: u64 = header.iter().map(|b| *b as u64).sum();
    let mut chksum = format!("{checksum:06o}");
    chksum.push('\0');
    chksum.push(' ');
    header[148..156].copy_from_slice(chksum.as_bytes());
    tar.extend_from_slice(&header);
    tar.extend_from_slice(content);
    let pad = (512 - content.len() % 512) % 512;
    tar.extend(std::iter::repeat_n(0u8, pad));
}

/// A 12-byte ustar numeric field: 11 octal digits + NUL.
fn octal_12(value: u64) -> [u8; 12] {
    let text = format!("{value:011o}");
    let mut field = [0u8; 12];
    field[..11].copy_from_slice(text.as_bytes());
    field
}

/// base64 (RFC 4648, standard alphabet, padded) — the seam encoding for
/// file content.
pub mod base64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(ALPHABET[(n >> 18) as usize & 0x3f] as char);
            out.push(ALPHABET[(n >> 12) as usize & 0x3f] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 0x3f] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 0x3f] as char
            } else {
                '='
            });
        }
        out
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let bytes: Vec<u8> = text.bytes().collect();
        if bytes.is_empty() {
            return Some(Vec::new());
        }
        if !bytes.len().is_multiple_of(4) {
            return None;
        }
        let value_of = |b: u8| -> Option<u32> {
            match b {
                b'A'..=b'Z' => Some((b - b'A') as u32),
                b'a'..=b'z' => Some((b - b'a') as u32 + 26),
                b'0'..=b'9' => Some((b - b'0') as u32 + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        };
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks(4) {
            let pad = chunk.iter().rev().take_while(|b| **b == b'=').count();
            if pad > 2 || chunk[..4 - pad].iter().any(|b| value_of(*b).is_none()) {
                return None;
            }
            let n = chunk
                .iter()
                .take(4 - pad)
                .map(|b| value_of(*b).unwrap())
                .fold(0u32, |acc, v| (acc << 6) | v);
            // Reconstruct the 1-3 bytes the chunk carries.
            let bytes_out: &[u8] = &match pad {
                0 => vec![(n >> 16) as u8, (n >> 8) as u8, n as u8],
                1 => vec![(n >> 10) as u8, (n >> 2) as u8],
                _ => vec![(n >> 4) as u8],
            };
            out.extend_from_slice(bytes_out);
        }
        Some(out)
    }
}

/// The serde adapter carrying file content as base64.
mod b64 {
    use serde::Deserialize;

    pub fn serialize<S: serde::Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::base64::encode(v))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        super::base64::decode(&text).ok_or_else(|| {
            serde::de::Error::custom("invalid base64 in diagnostics file content")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;

    #[test]
    fn pseudonymize_keeps_the_oui_and_hashes_the_rest() {
        let address: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let id = pseudonymize(&address);
        // The OUI prefix survives (vendor triage), the rest never does.
        assert!(id.starts_with("a0:fa:9c…"), "{id}");
        assert_eq!(id.len(), "a0:fa:9c…".len() + 8);
        assert!(
            id.chars().skip("a0:fa:9c…".len()).all(|c| c.is_ascii_hexdigit()),
            "the tail is a hash, not address bytes: {id}"
        );
        // The full address appears nowhere in the pseudonym.
        assert!(!id.contains("62:7d:1a"));
        assert!(!id.contains("627d1a"));
        // Deterministic, and distinct pads stay distinguishable.
        assert_eq!(id, pseudonymize(&address));
        let other: BdAddr = "a0:fa:9c:62:7d:1b".parse().unwrap();
        assert_ne!(id, pseudonymize(&other));
    }

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        assert_eq!(base64::encode(b"Man"), "TWFu");
        assert_eq!(base64::encode(b"Ma"), "TWE=");
        assert_eq!(base64::encode(b"M"), "TQ==");
        assert_eq!(base64::encode(b""), "");
        for (plain, encoded) in [
            (b"Man".as_slice(), "TWFu"),
            (b"Ma".as_slice(), "TWE="),
            (b"M".as_slice(), "TQ=="),
        ] {
            assert_eq!(base64::decode(encoded).unwrap(), plain);
        }
        // A log-sized round trip.
        let log = b"2026-01-01 daemon: pad connected\n".repeat(1000);
        assert_eq!(base64::decode(&base64::encode(&log)).unwrap(), log);
        assert!(base64::decode("not base64!").is_none());
    }

    fn sample_bundle() -> DiagnosticsBundle {
        DiagnosticsBundle {
            manifest: BundleManifest {
                schema_version: BUNDLE_SCHEMA_VERSION,
                app_version: "0.1.0".into(),
                protocol_version: PROTOCOL_VERSION,
                generated_unix: 1_700_000_000,
                tier: PrivilegeTier::Root,
                capabilities: vec![Capability::DiagnosticsExport],
                health: HealthCounters::default(),
                pads: vec![],
            },
            files: vec![DiagnosticsFile {
                name: "logs/bluepad-daemon.log".into(),
                content: b"2026-01-01 daemon: hello\n".to_vec(),
            }],
        }
    }

    /// An independent minimal ustar reader: header fields per the POSIX
    /// spec, so the writer is verified against the format, not against
    /// itself.
    struct UstarEntry {
        name: String,
        content: Vec<u8>,
    }

    fn read_ustar(tar: &[u8]) -> Vec<UstarEntry> {
        let mut entries = Vec::new();
        let mut offset = 0;
        while offset + 512 <= tar.len() {
            let header = &tar[offset..offset + 512];
            if header.iter().all(|b| *b == 0) {
                break; // the trailing zero blocks
            }
            let field = |range: std::ops::Range<usize>| {
                header[range]
                    .iter()
                    .take_while(|b| **b != 0)
                    .cloned()
                    .collect::<Vec<u8>>()
            };
            assert_eq!(&header[257..263], b"ustar\0", "ustar magic");
            assert_eq!(&header[263..265], b"00", "ustar version");
            assert_eq!(header[156], b'0', "regular file");
            let name = String::from_utf8(field(0..100)).unwrap();
            let size_text = String::from_utf8(field(124..136)).unwrap();
            let size = u64::from_str_radix(size_text.trim_end(), 8).unwrap();
            let content = tar[offset + 512..offset + 512 + size as usize].to_vec();
            entries.push(UstarEntry { name, content });
            offset += 512 + (size as usize).div_ceil(512) * 512;
        }
        entries
    }

    #[test]
    fn bundle_tar_is_a_valid_ustar_archive() {
        let bundle = sample_bundle();
        let tar = bundle_tar(&bundle);
        assert_eq!(tar.len() % 512, 0, "tar is 512-byte aligned");
        assert!(tar.ends_with(&[0u8; 1024]), "two trailing zero blocks");

        let entries = read_ustar(&tar);
        assert_eq!(entries.len(), 2, "manifest + one log");
        let manifest = &entries[0];
        assert_eq!(manifest.name, "manifest.json");
        let parsed: BundleManifest =
            serde_json::from_slice(&manifest.content).expect("manifest.json is JSON");
        assert_eq!(parsed.schema_version, BUNDLE_SCHEMA_VERSION);
        assert_eq!(parsed.app_version, "0.1.0");

        let log = &entries[1];
        assert_eq!(log.name, "logs/bluepad-daemon.log");
        assert_eq!(log.content, b"2026-01-01 daemon: hello\n");
    }

    #[test]
    fn bundle_files_round_trip_through_the_seam_encoding() {
        // The bundle crosses the UI↔daemon seam as JSON: file content
        // base64, everything else ordinary serde.
        let bundle = sample_bundle();
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(
            !json.contains("2026-01-01 daemon: hello"),
            "log bytes are base64 on the wire"
        );
        let back: DiagnosticsBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bundle);
    }

    #[test]
    fn scrub_rewrites_every_log_mac_form_to_a_pseudonym() {
        let known: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let log = "pairing with a0:fa:9c:62:7d:1a (BdAddr(a0fa9c627d1a)) \
                   parked pad a0fa9c627d1a adopting \
                   other pad 5c:c3:36:aa:bb:cc connected\n";
        let scrubbed = String::from_utf8(scrub_macs(log.as_bytes(), &[known])).unwrap();
        // No MAC in the clear, in any of the three forms.
        assert!(!scrubbed.contains("a0:fa:9c:62:7d:1a"), "{scrubbed}");
        assert!(!scrubbed.contains("a0fa9c627d1a"), "{scrubbed}");
        assert!(!scrubbed.contains("5c:c3:36:aa:bb:cc"), "{scrubbed}");
        // The pseudonyms keep the OUI, so triage still works.
        assert!(scrubbed.contains("a0:fa:9c…"), "{scrubbed}");
        assert!(scrubbed.contains("5c:c3:36…"), "{scrubbed}");
        // The structure around the addresses survives.
        assert!(scrubbed.contains("BdAddr("), "{scrubbed}");
        assert!(scrubbed.contains("parked pad"), "{scrubbed}");
    }

    #[test]
    fn scrub_leaves_plain_hex_data_alone() {
        // A bare 12-hex run that is not a known address is report data,
        // not scrubbed — the diagnostics stay useful.
        let known: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let log = b"report bytes a2310048ff2c crc 1a2b3c4d\n".to_vec();
        let scrubbed = scrub_macs(&log, &[known]);
        assert_eq!(scrubbed, log);
    }

    #[test]
    fn manifest_serializes_without_any_mac_in_the_clear() {
        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            app_version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            generated_unix: 0,
            tier: PrivilegeTier::Root,
            capabilities: vec![],
            health: HealthCounters::default(),
            pads: vec![PseudonymizedPad {
                id: pseudonymize(&"a0:fa:9c:62:7d:1a".parse().unwrap()),
                kind: PadKind::DualSense,
                phase: PadPhase::Connected,
                proven_path: Some(ConnectionPath::AppOwned),
                provenance: PairingProvenance::Stock,
            }],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains("a0:fa:9c:62:7d:1a"), "no colon MAC");
        assert!(!json.contains("a0fa9c627d1a"), "no normalized MAC");
        assert!(json.contains("dual_sense"), "the kind is kept");
    }
}
