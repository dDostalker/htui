//! Event dispatch with DOM-style bubbling.

use crate::tui::graph_api::RenderCmd;

use super::{EventBindings, EventType, HitRegion, eval::{EventContext, eval_handler}};

/// Dispatch `event_type` at canvas position `(x, y)` with bubbling.
///
/// Walks the target's `path` from the deepest element up to the root and
/// evaluates each element's `on*` handler body (a JavaScript-subset
/// expression; see [`eval`](crate::events::eval)). The first function
/// invocation that returns a [`RenderCmd`] short-circuits the walk; the
/// command is forwarded to the render thread by the input loop.
pub fn dispatch(
    region: &HitRegion,
    event_type: EventType,
    x: u32,
    y: u32,
    bindings: &mut EventBindings,
) -> Option<RenderCmd> {
    let target = region.path.first().and_then(|e| e.id.as_deref());
    for element in region.path.iter() {
        let Some(body) = element.actions.get(&event_type) else {
            continue;
        };
        let ctx = EventContext {
            event_type,
            target,
            current_target: element.id.as_deref(),
            path: &region.path,
            x,
            y,
        };
        if let Some(cmd) = eval_handler(body, &ctx, bindings) {
            return Some(cmd);
        }
    }
    None
}
