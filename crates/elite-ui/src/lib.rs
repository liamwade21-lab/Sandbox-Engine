//! Elite Dangerous's ship-UI rules as pure code (ARCHITECTURE.md A10): std and serde only; no Ruffle,
//! no Bevy, no GPU, no files. Each module has one job.

pub mod value;

pub mod facts;
pub mod feed;
pub mod text;

pub use value::Value;

pub use facts::UiFacts;
pub use text::Text;

// The cockpit side panels and the focus machine (task UI5a).
pub mod focus;
pub mod panel_facts;
pub mod panels;

pub use focus::Focus;
pub use panel_facts::PanelFacts;
