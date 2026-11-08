//! The versioned settings document (Memento, lightweight serde form).
//!
//! One document owned by the daemon: schema version, a global toggle, and
//! MAC-keyed per-pad overrides, with built-in defaults in code
//! (spec: Settings; controller-settings research §4.3). Crash safety by
//! atomic writes (temp + rename); an unparseable document is renamed `.bad`
//! and defaults take over (punktfunk precedent).

use crate::bdaddr::BdAddr;
use crate::kit::{Lightbar, PlayerIndex, Rumble};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

/// Lightbar color: follow the player color, or a fixed choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightbarMode {
    PlayerColor,
    Fixed,
}

/// Lightbar brightness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Brightness {
    Low,
    Medium,
    High,
}

/// DualSense player LEDs: follow the player index, or a manual override —
/// display-only, never changing the pad's index (spec: Settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerLedMode {
    Automatic,
    Manual,
}

/// Rumble intensity, scaling BluePad-initiated pulses only (game-driven
/// rumble is impossible through uhid on the 4.4 kernel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RumbleIntensity {
    Off,
    Low,
    Medium,
    High,
}

/// Per-pad settings (the 0.1.0 catalogue, spec: Settings).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PadSettings {
    pub lightbar_mode: LightbarMode,
    pub lightbar_color: Lightbar,
    pub lightbar_brightness: Brightness,
    pub player_leds: PlayerLedMode,
    pub player_leds_override: u8,
    pub rumble: RumbleIntensity,
    pub connection_feedback: bool,
}

impl Default for PadSettings {
    fn default() -> Self {
        PadSettings {
            lightbar_mode: LightbarMode::PlayerColor,
            lightbar_color: Lightbar {
                red: 0x00,
                green: 0x00,
                blue: 0x40,
            },
            lightbar_brightness: Brightness::Medium,
            player_leds: PlayerLedMode::Automatic,
            player_leds_override: 0,
            rumble: RumbleIntensity::Medium,
            connection_feedback: true,
        }
    }
}

impl PadSettings {
    /// The lightbar color to apply, given the pad's player index: the player
    /// color by default, the fixed choice on override, scaled by brightness.
    pub fn effective_lightbar(&self, player: PlayerIndex) -> Lightbar {
        let base = match self.lightbar_mode {
            LightbarMode::PlayerColor => crate::kit::player_lightbar_color(player),
            LightbarMode::Fixed => self.lightbar_color,
        };
        let factor = match self.lightbar_brightness {
            Brightness::Low => 0.4,
            Brightness::Medium => 0.7,
            Brightness::High => 1.0,
        };
        Lightbar {
            red: (base.red as f32 * factor) as u8,
            green: (base.green as f32 * factor) as u8,
            blue: (base.blue as f32 * factor) as u8,
        }
    }

    /// The player-LED mask to apply: the player-following pattern, or the
    /// manual override (display-only).
    pub fn effective_player_leds(&self, player: PlayerIndex) -> u8 {
        match self.player_leds {
            PlayerLedMode::Automatic => crate::kit::dualsense_player_led_mask(player),
            PlayerLedMode::Manual => self.player_leds_override,
        }
    }

    /// The rumble motors for a feedback pulse at this pad's intensity.
    pub fn pulse_rumble(&self) -> Rumble {
        let v = match self.rumble {
            RumbleIntensity::Off => 0,
            RumbleIntensity::Low => 0x40,
            RumbleIntensity::Medium => 0x99,
            RumbleIntensity::High => 0xFF,
        };
        Rumble {
            low: v,
            high: v,
            impulse_left: 0,
            impulse_right: 0,
        }
    }

    /// Whether feedback applies at all for this pad.
    pub fn feedback_enabled(&self) -> bool {
        self.connection_feedback && self.rumble != RumbleIntensity::Off
    }
}

/// The whole settings document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsDocument {
    pub schema_version: u32,
    /// The global toggle: when off, the daemon keeps no sessions.
    pub enabled: bool,
    /// MAC-keyed per-pad overrides (normalized lowercase hex).
    pub pads: BTreeMap<String, PadSettings>,
}

impl Default for SettingsDocument {
    fn default() -> Self {
        SettingsDocument {
            schema_version: SCHEMA_VERSION,
            enabled: true,
            pads: BTreeMap::new(),
        }
    }
}

/// The daemon-owned settings store: whole-document load/save, atomic writes,
/// `.bad` rename on unparseable documents, per-pad resolution with defaults.
pub struct SettingsStore {
    path: PathBuf,
    doc: SettingsDocument,
}

impl SettingsStore {
    /// Load the document; an unparseable or absent document falls back to
    /// defaults (the bad one is kept as `.bad` for inspection).
    pub fn load(path: impl Into<PathBuf>) -> SettingsStore {
        let path = path.into();
        let doc = fs::read_to_string(&path)
            .ok()
            .and_then(|text| match serde_json::from_str::<SettingsDocument>(&text) {
                Ok(doc) if doc.schema_version <= SCHEMA_VERSION => Some(doc),
                _ => None,
            })
            .unwrap_or_default();
        SettingsStore { path, doc }
    }

    /// Persist atomically: write to a temp file next to the target, then
    /// rename over it (Memento lightweight: atomic settings writes).
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&self.doc)?;
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }

    pub fn document(&self) -> &SettingsDocument {
        &self.doc
    }

    pub fn enabled(&self) -> bool {
        self.doc.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.doc.enabled = enabled;
    }

    /// Effective settings for a pad: its override, or built-in defaults.
    pub fn pad(&self, address: &BdAddr) -> PadSettings {
        self.doc
            .pads
            .get(&address.key())
            .cloned()
            .unwrap_or_default()
    }

    /// Live-apply a per-pad settings change (persists immediately).
    pub fn set_pad(&mut self, address: &BdAddr, settings: PadSettings) -> std::io::Result<()> {
        self.doc.pads.insert(address.key(), settings);
        self.save()
    }

    /// Forget is the only removal — and it deletes the pad's settings entry
    /// too (spec: Per-pad lifecycle).
    pub fn forget(&mut self, address: &BdAddr) -> std::io::Result<()> {
        self.doc.pads.remove(&address.key());
        self.save()
    }
}

/// Rename an unparseable document aside (the `.bad` policy), used by callers
/// that want the forensic copy kept even when `load` already fell back.
pub fn quarantine_bad_document(path: &Path) {
    let _ = fs::rename(path, path.with_extension("json.bad"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(name: &str, content: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bluepad-test-{name}-{}.json", std::process::id()));
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn absent_document_loads_defaults() {
        let store = SettingsStore::load("/nonexistent/bluepad/settings.json");
        assert!(store.enabled());
        assert_eq!(store.document().schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn unparseable_document_loads_defaults() {
        let p = tmpfile("bad", "{ not json ");
        let store = SettingsStore::load(&p);
        assert!(store.enabled());
        assert!(store.document().pads.is_empty());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn round_trips_per_pad_overrides() {
        let p = std::env::temp_dir().join(format!("bluepad-test-roundtrip-{}.json", std::process::id()));
        let addr: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();

        let mut store = SettingsStore::load(&p);
        let mut pad = PadSettings::default();
        pad.rumble = RumbleIntensity::High;
        pad.lightbar_mode = LightbarMode::Fixed;
        pad.lightbar_color = Lightbar { red: 0xFF, green: 0, blue: 0 };
        store.set_pad(&addr, pad).unwrap();

        // A fresh store reads the same values back.
        let reloaded = SettingsStore::load(&p);
        assert_eq!(reloaded.pad(&addr).rumble, RumbleIntensity::High);
        assert_eq!(reloaded.pad(&addr).lightbar_color.red, 0xFF);
        // Unknown pads get defaults.
        let other: BdAddr = "01:02:03:04:05:06".parse().unwrap();
        assert_eq!(reloaded.pad(&other), PadSettings::default());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn forget_removes_the_settings_entry() {
        let p = std::env::temp_dir().join(format!("bluepad-test-forget-{}.json", std::process::id()));
        let addr: BdAddr = "a0:fa:9c:62:7d:1a".parse().unwrap();
        let mut store = SettingsStore::load(&p);
        store.set_pad(&addr, PadSettings::default()).unwrap();
        store.forget(&addr).unwrap();
        assert!(!store.document().pads.contains_key(&addr.key()));
        assert_eq!(SettingsStore::load(&p).pad(&addr), PadSettings::default());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn unknown_keys_are_ignored_for_forward_compat() {
        let p = tmpfile("fwd", r#"{"schema_version":1,"enabled":false,"pads":{},"future_knob":42}"#);
        let store = SettingsStore::load(&p);
        assert!(!store.enabled());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn save_is_atomic_no_tmp_left_behind() {
        let p = std::env::temp_dir().join(format!("bluepad-test-atomic-{}.json", std::process::id()));
        let mut store = SettingsStore::load(&p);
        store.set_enabled(false);
        store.save().unwrap();
        assert!(p.exists());
        assert!(!p.with_extension("json.tmp").exists());
        assert!(!SettingsStore::load(&p).enabled());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn player_following_defaults_and_overrides() {
        let mut s = PadSettings::default();
        // Automatic player LEDs follow the index.
        assert_eq!(s.effective_player_leds(PlayerIndex::P1), 0x04);
        assert_eq!(s.effective_lightbar(PlayerIndex::P1).blue, 44); // 0x40 * 0.7
        // Manual override changes only what the pad displays.
        s.player_leds = PlayerLedMode::Manual;
        s.player_leds_override = 0x1F;
        assert_eq!(s.effective_player_leds(PlayerIndex::P3), 0x1F);
        // Brightness scales the lightbar.
        s.lightbar_brightness = Brightness::High;
        s.lightbar_mode = LightbarMode::Fixed;
        s.lightbar_color = Lightbar { red: 0xFF, green: 0, blue: 0 };
        assert_eq!(s.effective_lightbar(PlayerIndex::P2).red, 0xFF);
        s.lightbar_brightness = Brightness::Low;
        assert_eq!(s.effective_lightbar(PlayerIndex::P2).red, 0x66);
    }

    #[test]
    fn rumble_intensity_scales_pulses() {
        let mut s = PadSettings::default();
        s.rumble = RumbleIntensity::Off;
        assert!(!s.feedback_enabled());
        assert_eq!(s.pulse_rumble().low, 0);
        s.rumble = RumbleIntensity::High;
        assert_eq!(s.pulse_rumble().low, 0xFF);
        assert!(s.feedback_enabled());
    }
}
