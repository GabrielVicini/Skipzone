//! Controls that float directly over the map, anchored to the screen corners.
//!
//! Each group is its own `Area`, so it sits above the map layer and takes
//! pointer input before the map does - dragging a slider never pans the map
//! underneath. No group draws a backing panel: only the individual controls
//! carry the translucent [`crate::ui::theme::floating_frame`] chrome that keeps
//! them legible over bright tiles.

mod controls;
mod map_tools;
mod time_date;

use egui::{Align, Align2, Area, Context, Id, InnerResponse, Order, Rect, Ui, Vec2, pos2};

use crate::state::{Session, UiState};
use crate::ui::actions::Action;

use super::map::MapView;

/// Gap between a floating group and the window edge.
const MARGIN: f32 = 12.0;

/// Draw every floating group. Returns the first action any of them produced.
pub fn draw(
    ctx: &Context,
    session: &mut Session,
    ui_state: &mut UiState,
    map: &MapView,
) -> Option<Action> {
    // The map's visible area: the window minus the header and, when it is open,
    // the trace panel. Everything floats inside this, so opening the panel
    // slides the right-hand groups across instead of hiding them behind it.
    let over_map = {
        let screen = ctx.viewport_rect();
        Rect::from_min_max(
            pos2(screen.left(), screen.top() + ui_state.header_height),
            pos2(screen.right() - ui_state.right_inset, screen.bottom()),
        )
    };

    // Right edge, starting just below the solid header.
    let controls = corner_area(ctx, "overlay_controls", over_map, Align2::RIGHT_TOP, |ui| {
        controls::overlay(ui, session, ui_state)
    });

    // Bottom left: time and date.
    corner_area(
        ctx,
        "overlay_time_date",
        over_map,
        Align2::LEFT_BOTTOM,
        |ui| time_date::overlay(ui, session, ui_state),
    );

    // Bottom right: map framing.
    let tools = corner_area(
        ctx,
        "overlay_map_tools",
        over_map,
        Align2::RIGHT_BOTTOM,
        |ui| map_tools::overlay(ui, ui_state, map.zoom()),
    );

    controls.inner.or(tools.inner)
}

/// An `Area` pinned to one corner of `bounds`, inset by [`MARGIN`].
///
/// The corner is resolved here rather than with `Area::anchor` because the
/// group's own size is needed to place its right/bottom edge, and that is only
/// known once it has been laid out. The size measured last frame is read back
/// from egui's memory and the top-left corner computed from it, so every area
/// is positioned by a plain top-left `fixed_pos`. On the very first frame the
/// size is unknown and the group sits at the edge; it settles on the next.
fn corner_area<R>(
    ctx: &Context,
    id: &str,
    bounds: Rect,
    corner: Align2,
    content: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let id = Id::new(id);
    let size = ctx
        .memory(|memory| memory.area_rect(id))
        .map_or(Vec2::ZERO, |rect| rect.size());
    let inner = bounds.shrink(MARGIN);
    let x = match corner.x() {
        Align::Min | Align::Center => inner.left(),
        Align::Max => inner.right() - size.x,
    };
    let y = match corner.y() {
        Align::Min | Align::Center => inner.top(),
        Align::Max => inner.bottom() - size.y,
    };
    Area::new(id)
        .order(Order::Middle)
        .fixed_pos(pos2(x, y))
        .show(ctx, content)
}
