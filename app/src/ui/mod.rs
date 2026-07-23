//! Everything egui, arranged by where it appears on screen.
//!
//! * [`shell`]    - the layout: what goes where.
//! * [`header`]   - the solid top bar (menus, status, TX/RX rows).
//! * [`map`]      - the full-screen map and its drawing overlays.
//! * [`overlays`] - the control groups floating over the map.
//! * [`modals`]   - the dialogs (trace, best frequency, settings, about).
//! * [`panels`]   - the trace readouts, composed inside the trace dialog.
//! * [`widgets`]  - reusable pieces: charts, menus, the calendar, form bits.
//! * [`theme`]    - colours, spacing and container chrome, defined once.
//! * [`actions`]  - what the interface can be asked to do, in one enum.
//!
//! The domain modules (`scenario`, `solve`, `sweep`, `noise`) have no egui
//! dependency, and nothing here computes anything the solver could compute.

pub mod actions;
pub mod header;
pub mod map;
pub mod modals;
pub mod overlays;
pub mod panels;
pub mod shell;
pub mod theme;
pub mod widgets;
