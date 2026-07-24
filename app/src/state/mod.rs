//! Application state, split by lifetime and owner.
//!
//! * [`Session`] - the scenario and everything computed from it, plus the
//!   handle to the background solver. No egui types.
//! * [`UiState`] - what the interface remembers between frames: open dialogs,
//!   entry buffers, the notation each station is being typed in.
//! * [`LocationEntry`] - the text buffers for one station's position.
//!
//! Widgets take these by reference and hold nothing themselves, so any control
//! can be moved between the header, an overlay or a dialog without carrying
//! state with it.

mod location;
mod session;
mod ui;

pub use location::{LocationEntry, LocationMode};
pub use session::{Busy, Session, SolveResults};
pub use ui::{CalendarState, Menu, UiState};
