//! The per-kind pad kit: authored descriptor, report parser, output writer.
//!
//! The kit is the Abstract Factory's product family (design-patterns research,
//! maintainer ruling): a per-kind × per-path bundle whose members are only
//! fully meaningful on the app-owned session — stock-path kits are degenerate
//! (connect-only) by design, and an unsupported kind × path combination is a
//! first-class product.

mod crc32;
mod descriptor;
mod output;
mod parse;

pub use descriptor::{DescriptorDirector, HidDescriptorBuilder};
pub use output::{dualsense_player_led_mask, player_lightbar_color};
pub use parse::{Battery, PadInput, Reading};

use crate::pad::{ConnectionPath, PadKind};
use serde::{Deserialize, Serialize};

/// A standard gamepad button, at its authored descriptor ordinal.
///
/// Ordinals are the load-bearing choice: kernel 4.4 `hid-input` maps
/// Gamepad-application button *n* to `BTN_GAMEPAD + (n-1)`, and the authored
/// ordinals are picked so every control lands on a code the streaming apps'
/// SDL forks auto-map (input-delivery research §4.2–§4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Cross = 1,
    Circle = 2,
    Triangle = 4,
    Square = 5,
    L1 = 7,
    R1 = 8,
    Create = 11,
    Options = 12,
    Guide = 13,
    L3 = 14,
    R3 = 15,
}

impl Button {
    /// The authored descriptor ordinal (1-based).
    pub fn ordinal(self) -> u8 {
        self as u8
    }

    /// The bit in the authored report's button field.
    pub fn bit(self) -> u16 {
        1u16 << (self.ordinal() - 1)
    }

    pub fn label(self) -> &'static str {
        match self {
            Button::Cross => "Cross",
            Button::Circle => "Circle",
            Button::Triangle => "Triangle",
            Button::Square => "Square",
            Button::L1 => "L1",
            Button::R1 => "R1",
            Button::Create => "Create",
            Button::Options => "Options",
            Button::Guide => "PS",
            Button::L3 => "L3",
            Button::R3 => "R3",
        }
    }
}

/// The authored ordinals that intentionally land on codes no SDL auto-mapping
/// reads (spares keep the real controls on their target ordinals).
pub const SPARE_ORDINALS: [u8; 4] = [3, 6, 9, 10];

/// Kernel 4.4 `BTN_GAMEPAD` (== `BTN_SOUTH`), the base of the button range.
const BTN_GAMEPAD: u16 = 0x130;

/// The authored report's axis → kernel-code assignment.
///
/// Codes are the kernel 4.4 contract for the descriptor's Generic Desktop
/// usages (linux v4.4 drivers/hid/hid-input.c + input-event-codes.h):
/// GD_X→ABS_X … GD_Rz→ABS_RZ, dpad usages→ABS_HAT0X/Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisCodes {
    pub left_stick: (u16, u16),
    pub right_stick: (u16, u16),
    pub triggers: (u16, u16),
}

const AXIS_CODES: AxisCodes = AxisCodes {
    left_stick: (0x00, 0x01),  // ABS_X, ABS_Y
    right_stick: (0x03, 0x04), // ABS_RX, ABS_RY
    triggers: (0x02, 0x05),    // ABS_Z (L2), ABS_RZ (R2)
};

/// A rumble request. `low`/`high` are the classic motors; the impulse fields
/// apply only where the pad has them (Xbox One S trigger rumble).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rumble {
    pub low: u8,
    pub high: u8,
    pub impulse_left: u8,
    pub impulse_right: u8,
}

/// A lightbar color request (already brightness-scaled by the settings layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lightbar {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// A request for pad-side output, built by the settings layer and encoded by
/// the kit. Fields the pad kind cannot honor are dropped, never fabricated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputRequest {
    pub rumble: Option<Rumble>,
    pub lightbar: Option<Lightbar>,
    /// DualSense player-LED mask (5 bits).
    pub player_leds: Option<u8>,
}

/// The connect-order slot (1–4) the daemon assigns automatically
/// (glossary: Player index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerIndex {
    P1,
    P2,
    P3,
    P4,
}

impl PlayerIndex {
    pub fn from_slot(slot: usize) -> Self {
        match slot {
            0 => PlayerIndex::P1,
            1 => PlayerIndex::P2,
            2 => PlayerIndex::P3,
            _ => PlayerIndex::P4,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PlayerIndex::P1 => "Player 1",
            PlayerIndex::P2 => "Player 2",
            PlayerIndex::P3 => "Player 3",
            PlayerIndex::P4 => "Player 4",
        }
    }
}

/// The per-kind × per-path kit (Abstract Factory product).
pub struct PadKit {
    kind: PadKind,
    path: ConnectionPath,
    descriptor: Vec<u8>,
    parser: Option<parse::RawParser>,
    output: Option<output::OutputEncoder>,
}

impl PadKit {
    /// The authored HID report descriptor fed to `uhid`. Empty on the stock
    /// path (LG's stack owns the session there).
    pub fn descriptor(&self) -> &[u8] {
        &self.descriptor
    }

    /// Parse a raw pad input report into named control states.
    /// `None` for stock kits or unknown/truncated reports.
    pub fn parse_raw(&self, raw: &[u8]) -> Option<PadInput> {
        self.parser.and_then(|p| p(raw))
    }

    /// Encode an output report for this pad, or `None` if the kit has no
    /// output surface (stock path).
    pub fn build_output(&self, request: &OutputRequest, seq: u8) -> Option<Vec<u8>> {
        self.output.map(|e| e(request, seq))
    }

    /// The kernel evdev code an authored button lands on.
    pub fn kernel_code(&self, button: Button) -> Option<u16> {
        let _ = self;
        Some(BTN_GAMEPAD + button.ordinal() as u16 - 1)
    }

    /// The codes the spare ordinals land on (no SDL auto-mapping reads them).
    pub fn spare_codes(&self) -> Vec<u16> {
        SPARE_ORDINALS
            .iter()
            .map(|n| BTN_GAMEPAD + *n as u16 - 1)
            .collect()
    }

    /// D-pad codes (hat 0).
    pub fn hat_codes(&self) -> (u16, u16) {
        (0x10, 0x11) // ABS_HAT0X, ABS_HAT0Y
    }

    /// Stick and trigger axis codes.
    pub fn axis_codes(&self) -> AxisCodes {
        AXIS_CODES
    }

    /// Encode the authored `uhid` input report for a parsed pad state.
    pub fn encode_authored_report(&self, input: &PadInput) -> Vec<u8> {
        let mut out = vec![0u8; 16];
        out[0] = 1; // report id
        out[1..3].copy_from_slice(&input.lx.to_le_bytes());
        out[3..5].copy_from_slice(&input.ly.to_le_bytes());
        out[5..7].copy_from_slice(&input.l2.to_le_bytes());
        out[7..9].copy_from_slice(&input.r2.to_le_bytes());
        out[9..11].copy_from_slice(&input.rx.to_le_bytes());
        out[11..13].copy_from_slice(&input.ry.to_le_bytes());
        out[13] = input.hat & 0x0F;
        out[14..16].copy_from_slice(&input.buttons.to_le_bytes());
        out
    }

    /// Every control with its interpreted label and raw code side by side —
    /// the pad diagnostic's data (spec: Pad diagnostic).
    pub fn diagnostic_readings(&self, input: &PadInput) -> Vec<Reading> {
        let mut out = vec![];
        for button in [
            Button::Cross,
            Button::Circle,
            Button::Square,
            Button::Triangle,
            Button::L1,
            Button::R1,
            Button::Create,
            Button::Options,
            Button::Guide,
            Button::L3,
            Button::R3,
        ] {
            out.push(Reading {
                label: button.label(),
                code: self.kernel_code(button),
                value: u16::from(input.pressed(button)),
            });
        }
        let (hat_x, hat_y) = self.hat_codes();
        for (label, code, value) in [
            ("D-pad up", hat_y, u16::from(input.hat == 0)),
            ("D-pad down", hat_y, u16::from(input.hat == 4)),
            ("D-pad left", hat_x, u16::from(input.hat == 6)),
            ("D-pad right", hat_x, u16::from(input.hat == 2)),
        ] {
            out.push(Reading {
                label,
                code: Some(code),
                value,
            });
        }
        let a = self.axis_codes();
        for (label, code, value) in [
            ("Left stick X", a.left_stick.0, input.lx),
            ("Left stick Y", a.left_stick.1, input.ly),
            ("Right stick X", a.right_stick.0, input.rx),
            ("Right stick Y", a.right_stick.1, input.ry),
            ("L2", a.triggers.0, input.l2),
            ("R2", a.triggers.1, input.r2),
        ] {
            out.push(Reading {
                label,
                code: Some(code),
                value,
            });
        }
        if let Some(battery) = input.battery {
            out.push(Reading {
                label: "Battery",
                code: None,
                value: battery.percent as u16,
            });
        }
        out
    }

    pub fn kind(&self) -> PadKind {
        self.kind
    }

    pub fn path(&self) -> ConnectionPath {
        self.path
    }
}

/// The kit factory: `kit(kind, path)` models the kind × path compatibility
/// matrix; `None` is the first-class "unsupported combination" product.
pub trait PadKitFactory {
    fn kit(&self, kind: PadKind, path: ConnectionPath) -> Option<PadKit>;
}

/// The stock kit factory (Abstract Factory). Stock kits are degenerate:
/// no descriptor, no parser, no output.
struct StockKitFactory;

impl PadKitFactory for StockKitFactory {
    fn kit(&self, kind: PadKind, path: ConnectionPath) -> Option<PadKit> {
        match (kind, path) {
            // Proven for pre-2023 DualSense; best-effort DS4; generic pads get
            // truthful connect-only with no tier. Xbox One S over the stock
            // path is unproven — not offered.
            (PadKind::DualSense | PadKind::Ds4 | PadKind::GenericHid, ConnectionPath::Stock) => {
                Some(PadKit {
                    kind,
                    path,
                    descriptor: vec![],
                    parser: None,
                    output: None,
                })
            }
            _ => None,
        }
    }
}

/// The default kit factory for 0.1.0.
pub fn factory() -> DefaultKitFactory {
    DefaultKitFactory
}

pub struct DefaultKitFactory;

impl PadKitFactory for DefaultKitFactory {
    fn kit(&self, kind: PadKind, path: ConnectionPath) -> Option<PadKit> {
        match (kind, path) {
            (PadKind::DualSense | PadKind::Ds4 | PadKind::XboxOneS, ConnectionPath::AppOwned) => {
                Some(PadKit {
                    kind,
                    path,
                    descriptor: DescriptorDirector::new().authored_layout(kind),
                    parser: Some(parse::parser_for(kind)),
                    output: Some(output::encoder_for(kind)),
                })
            }
            (_, ConnectionPath::Stock) => StockKitFactory.kit(kind, path),
            _ => None,
        }
    }
}
