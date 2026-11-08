//! The authored HID report descriptor (Builder + Director).
//!
//! The descriptor is the load-bearing artifact of the app-owned session: its
//! button ordinals are chosen so kernel 4.4 `hid-input` lands every control
//! on the evdev codes the streaming apps' SDL forks auto-map
//! (input-delivery research §4.2–§4.3). One authored layout serves every
//! pad kind in 0.1.0 — the per-kind difference is the report *parse*, not the
//! delivery.

/// The Director: per-kind recipes for the authored descriptor.
pub struct DescriptorDirector;

impl DescriptorDirector {
    pub fn new() -> Self {
        DescriptorDirector
    }

    /// The authored descriptor for a pad kind on the app-owned session.
    pub fn authored_layout(&self, _kind: crate::pad::PadKind) -> Vec<u8> {
        // Guardrail from the design-patterns ruling: "if one fixed descriptor
        // turns out to serve all pads, collapse back to a const table". In
        // 0.1.0 every kind shares the standard gamepad recipe; the Director
        // stays the single place a per-kind recipe would diverge.
        self.standard_gamepad()
    }

    /// The standard gamepad recipe: 6×16-bit axes (X, Y, Z, Rz, Rx, Ry),
    /// a 4-bit null-state hat, and 15 1-bit buttons — in that report order.
    fn standard_gamepad(&self) -> Vec<u8> {
        let mut b = HidDescriptorBuilder::gamepad();
        b.axis(GdUsage::X)
            .axis(GdUsage::Y)
            .axis(GdUsage::Z) // L2 trigger
            .axis(GdUsage::Rz) // R2 trigger
            .axis(GdUsage::Rx)
            .axis(GdUsage::Ry)
            .hat_switch()
            .buttons(15);
        b.build()
    }
}

impl Default for DescriptorDirector {
    fn default() -> Self {
        Self::new()
    }
}

/// Generic Desktop usages used by the authored descriptor.
#[derive(Clone, Copy)]
pub enum GdUsage {
    X = 0x30,
    Y = 0x31,
    Z = 0x32,
    Rx = 0x33,
    Ry = 0x34,
    Rz = 0x35,
}

/// HID descriptor byte builder. Field methods emit standard short items;
/// `build()` closes the collections and returns the descriptor bytes.
///
/// This is the full Builder form committed by the design-patterns ruling,
/// scoped to HID descriptor/report-shaped data only.
pub struct HidDescriptorBuilder {
    items: Vec<u8>,
    open_collections: usize,
}

impl HidDescriptorBuilder {
    /// Start a Gamepad application collection.
    pub fn gamepad() -> Self {
        let mut items = Vec::new();
        items.extend_from_slice(&[0x05, 0x01]); // Usage Page (Generic Desktop)
        items.extend_from_slice(&[0x09, 0x05]); // Usage (Gamepad)
        items.extend_from_slice(&[0xA1, 0x01]); // Collection (Application)
        items.extend_from_slice(&[0x85, 0x01]); // Report ID (1)
        items.extend_from_slice(&[0xA1, 0x00]); // Collection (Physical)
        HidDescriptorBuilder {
            items,
            open_collections: 2,
        }
    }

    /// A 16-bit absolute axis with logical range 0..=65535.
    pub fn axis(&mut self, usage: GdUsage) -> &mut Self {
        self.items.extend_from_slice(&[0x05, 0x01]); // Usage Page (Generic Desktop)
        self.items.extend_from_slice(&[0x09, usage as u8]); // Usage (n)
        self.items.extend_from_slice(&[0x15, 0x00]); // Logical Minimum (0)
        self.items.extend_from_slice(&[0x27, 0xFF, 0xFF, 0x00, 0x00]); // Logical Maximum (65535)
        self.items.extend_from_slice(&[0x75, 0x10]); // Report Size (16)
        self.items.extend_from_slice(&[0x95, 0x01]); // Report Count (1)
        self.items.extend_from_slice(&[0x81, 0x02]); // Input (Data, Var, Abs)
        self
    }

    /// A 4-bit null-state hat switch (D-pad), padded to a byte.
    pub fn hat_switch(&mut self) -> &mut Self {
        self.items.extend_from_slice(&[0x05, 0x01]); // Usage Page (Generic Desktop)
        self.items.extend_from_slice(&[0x09, 0x39]); // Usage (Hat switch)
        self.items.extend_from_slice(&[0x15, 0x00]); // Logical Minimum (0)
        self.items.extend_from_slice(&[0x25, 0x07]); // Logical Maximum (7)
        self.items.extend_from_slice(&[0x35, 0x00]); // Physical Minimum (0)
        self.items.extend_from_slice(&[0x46, 0x3B, 0x01]); // Physical Maximum (315)
        self.items.extend_from_slice(&[0x65, 0x14]); // Unit (Eng Rot)
        self.items.extend_from_slice(&[0x75, 0x04]); // Report Size (4)
        self.items.extend_from_slice(&[0x95, 0x01]); // Report Count (1)
        self.items.extend_from_slice(&[0x81, 0x42]); // Input (Data, Var, Abs, Null State)
        self.items.extend_from_slice(&[0x65, 0x00]); // Unit (None)
        self.padding(4);
        self
    }

    /// `count` 1-bit buttons, usages 1..=count, padded to a byte.
    pub fn buttons(&mut self, count: u8) -> &mut Self {
        self.items.extend_from_slice(&[0x05, 0x09]); // Usage Page (Button)
        self.items.extend_from_slice(&[0x19, 0x01]); // Usage Minimum (1)
        self.items.extend_from_slice(&[0x29, count]); // Usage Maximum (count)
        self.items.extend_from_slice(&[0x15, 0x00]); // Logical Minimum (0)
        self.items.extend_from_slice(&[0x25, 0x01]); // Logical Maximum (1)
        self.items.extend_from_slice(&[0x75, 0x01]); // Report Size (1)
        self.items.extend_from_slice(&[0x95, count]); // Report Count (count)
        self.items.extend_from_slice(&[0x81, 0x02]); // Input (Data, Var, Abs)
        let pad = (8 - (count % 8)) % 8;
        if pad > 0 {
            self.padding(pad as usize);
        }
        self
    }

    /// Constant padding bits.
    pub fn padding(&mut self, bits: usize) -> &mut Self {
        self.items.extend_from_slice(&[0x75, bits as u8]); // Report Size (bits)
        self.items.extend_from_slice(&[0x95, 0x01]); // Report Count (1)
        self.items.extend_from_slice(&[0x81, 0x03]); // Input (Const, Var, Abs)
        self
    }

    /// Close collections and return the descriptor bytes.
    pub fn build(&mut self) -> Vec<u8> {
        for _ in 0..self.open_collections {
            self.items.extend_from_slice(&[0xC0]); // End Collection
        }
        self.open_collections = 0;
        self.items.clone()
    }
}
