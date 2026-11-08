//! bluepad-radio: the radio-transport implementations behind the core
//! traits (the Bridge's implementation hierarchy).
//!
//! - `fake`: scripted transports for the connection-layer test seam
//! - `l2cap`: the app-owned session over raw kernel L2CAP (Linux)
//! - `uhid`: the authored virtual input device through /dev/uhid
//! - `ls2`: the stock-path adapter over LG's bluetooth2 (on-device)

pub mod fake;
pub mod l2cap;
pub mod ls2;
pub mod uhid;
