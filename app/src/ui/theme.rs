//! Shared colour palette for the debug readouts and status chips.

use egui::Color32;

pub const OK: Color32 = Color32::from_rgb(0x1B, 0x7F, 0x3B);
pub const WARN: Color32 = Color32::from_rgb(0xC8, 0x7A, 0x00);
pub const BAD: Color32 = Color32::from_rgb(0xC8, 0x3A, 0x1C);
pub const FAIL: Color32 = Color32::from_rgb(0xD0, 0x21, 0x1C);
pub const MUTED: Color32 = Color32::from_gray(0x88);
