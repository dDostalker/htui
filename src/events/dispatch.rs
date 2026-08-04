//! Event dispatch with DOM-style bubbling.

use crate::tui::graph_api::RenderCmd;

use super::{EventBindings, EventType, HitRegion, HtmlEvent, eval::{EventContext, eval_handler}};

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

/// Invoke a registered action directly with a synthetic event.
///
/// Backs the periodic auto-refresh: no pointer position exists, so the event
/// carries empty `target` / `current_target` / `path` fields and canvas
/// coordinates `(0, 0)`. Returns the handler's [`RenderCmd`], or `None` when
/// the action is not registered.
pub fn invoke_action(
    name: &str,
    bindings: &mut EventBindings,
    event_type: EventType,
) -> Option<RenderCmd> {
    let handler = bindings.actions.get_mut(name)?;
    let event = HtmlEvent {
        event_type,
        target: None,
        current_target: None,
        path: &[],
        action: Some(name),
        x: 0,
        y: 0,
        bubbles: true,
    };
    handler(&event)
}
