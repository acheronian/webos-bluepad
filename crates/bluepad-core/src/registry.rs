//! The pad registry: the daemon's persistent record of every paired pad.
//!
//! Outlives any individual connection; Forget is the only removal
//! (glossary: Pad registry). Reconcile-not-restore: on wake the daemon
//! rebuilds its view from the registry and the radio's actual state, never
//! from a snapshot of what used to be connected.

use crate::bdaddr::BdAddr;
use crate::pad::{ConnectionPath, PadIdentity, PadKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingProvenance {
    /// Trust established through the TV's stock stack (0.1.0's only path;
    /// app-owned pairing is gated behind the key-access spike).
    Stock,
}

/// One registry entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub identity: PadIdentity,
    pub kind: PadKind,
    /// The connection path this pad has proven to work; a known pad's
    /// proven path sticks (spec: Connection strategy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proven_path: Option<ConnectionPath>,
    pub provenance: PairingProvenance,
}

/// The registry store: one JSON document, atomic writes.
pub struct PadRegistry {
    path: PathBuf,
    entries: BTreeMap<String, RegistryEntry>,
}

impl PadRegistry {
    pub fn load(path: impl Into<PathBuf>) -> PadRegistry {
        let path = path.into();
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<BTreeMap<String, RegistryEntry>>(&text).ok())
            .unwrap_or_default();
        PadRegistry { path, entries }
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&self.entries)?)?;
        fs::rename(&tmp, &self.path)
    }

    pub fn entries(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.entries.values()
    }

    pub fn get(&self, address: &BdAddr) -> Option<&RegistryEntry> {
        self.entries.get(&address.key())
    }

    pub fn upsert(&mut self, entry: RegistryEntry) -> std::io::Result<()> {
        self.entries
            .insert(entry.identity.address.key(), entry);
        self.save()
    }

    /// Record that a path worked for this pad (routing table stickiness).
    pub fn record_proven_path(
        &mut self,
        address: &BdAddr,
        path: ConnectionPath,
    ) -> std::io::Result<()> {
        if let Some(entry) = self.entries.get_mut(&address.key()) {
            entry.proven_path = Some(path);
            self.save()
        } else {
            Ok(())
        }
    }

    /// Forget: the only removal.
    pub fn forget(&mut self, address: &BdAddr) -> std::io::Result<()> {
        self.entries.remove(&address.key());
        self.save()
    }

    /// The routing decision for a pad: its proven path sticks; new pads try
    /// app-owned first (spec: Connection strategy).
    pub fn route_for(&self, address: &BdAddr) -> ConnectionPath {
        self.get(address)
            .and_then(|e| e.proven_path)
            .unwrap_or(ConnectionPath::AppOwned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(addr: &str, name: &str) -> PadIdentity {
        PadIdentity {
            address: addr.parse().unwrap(),
            name: name.into(),
            vid: Some(0x054c),
            pid: Some(0x0ce6),
        }
    }

    fn entry(addr: &str) -> RegistryEntry {
        RegistryEntry {
            identity: identity(addr, "DualSense Wireless Controller"),
            kind: PadKind::DualSense,
            proven_path: None,
            provenance: PairingProvenance::Stock,
        }
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bluepad-registry-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn round_trips_entries_and_survives_reload() {
        let p = tmp("roundtrip");
        let mut reg = PadRegistry::load(&p);
        reg.upsert(entry("a0:fa:9c:62:7d:1a")).unwrap();
        let reloaded = PadRegistry::load(&p);
        assert_eq!(reloaded.entries().count(), 1);
        assert_eq!(
            reloaded.get(&"a0:fa:9c:62:7d:1a".parse().unwrap()).unwrap().kind,
            PadKind::DualSense
        );
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn proven_path_sticks_as_the_route() {
        let p = tmp("route");
        let mut reg = PadRegistry::load(&p);
        let addr: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        reg.upsert(entry("a0:fa:9c:62:7d:1a")).unwrap();
        // New pads try app-owned first.
        assert_eq!(reg.route_for(&addr), ConnectionPath::AppOwned);
        // Once a path is proven, it sticks.
        reg.record_proven_path(&addr, ConnectionPath::Stock).unwrap();
        assert_eq!(reg.route_for(&addr), ConnectionPath::Stock);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn forget_is_the_only_removal() {
        let p = tmp("forget");
        let mut reg = PadRegistry::load(&p);
        let addr: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        reg.upsert(entry("a0:fa:9c:62:7d:1a")).unwrap();
        reg.forget(&addr).unwrap();
        assert!(reg.get(&addr).is_none());
        assert!(PadRegistry::load(&p).entries().count() == 0);
        let _ = fs::remove_file(&p);
    }
}
