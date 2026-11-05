//! Everything egui, arranged by where it appears on screen.
//!
//! * [`shell`]    - the layout: what goes where.
//! * [`header`]   - the solid top bar (menus, status, TX/RX rows).
//! * [`map`]      - the full-screen map and its drawing overlays.
//! * [`overlays`] - the control groups floating over the map.
//! * [`trace_panel`] - the docked Calculate readout.
//! * [`modals`]   - the dialog windows (best frequency, coverage, settings,
//!   antennas, about).
//! * [`panels`]   - the trace readouts, composed inside the trace dialog.
//! * [`widgets`]  - reusable pieces: charts, menus, the calendar, form bits.
//! * [`theme`]    - colours, spacing and container chrome, defined once.
//! * [`actions`]  - what the interface can be asked to do, in one enum.
//! * [`scratch`]  - what the views remember between frames: text buffers,
//!   the open menu, layout sizes, the basemap texture.
//!
//! Every view draws from `&AppState` and pushes [`actions::Action`]s; none
//! can change the state. What a view must remember between frames lives in
//! the [`scratch::ViewScratch`] it is lent.
//!
//! The domain modules (`scenario`, `solve`, `sweep`, `noise`) have no egui
//! dependency, and nothing here computes anything the solver could compute.

pub mod actions;
pub mod header;
pub mod map;
pub mod modals;
pub mod overlays;
pub mod panels;
pub mod scratch;
pub mod shell;
pub mod theme;
pub mod trace_panel;
pub mod widgets;
