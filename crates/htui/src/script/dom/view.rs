//! Window-level view API: viewport getters (`innerWidth` / `innerHeight` /
//! `scrollY` / …), `matchMedia`, `scrollTo` / `scrollBy` and the
//! `window.addEventListener` form.
//!
//! `window` is an alias of `globalThis`, which has no node identity — so
//! window-level event listeners live under the reserved `"window"` key of the
//! same `__htuiListeners` registry the element wrappers use (node keys are
//! numeric strings, so the two key spaces never collide).

use boa_engine::{
    Context, JsNativeError, JsResult, JsValue, NativeFunction, js_string,
    object::ObjectInitializer,
    object::builtins::JsArray,
    property::{Attribute, PropertyDescriptorBuilder},
};

use crate::html::css_parse::{Viewport, parse_media_query_list};
use crate::layout::Rect;
use crate::tdom::node::NodeId;
use crate::tui::graph_api::{RenderCmd, ScrollIntoViewBlock, ScrollTarget};

use super::super::{selection_text, with_focused, with_hit_map, with_ui};
use super::{js_fn, this_node_id};

/// Registry key holding the window-level listeners.
const WINDOW_KEY: &str = "window";

/// `element.scrollIntoView([arg])` — scroll the page so this element lands at
/// the requested alignment. `arg` is `true` (`'start'`), `false` (`'end'`) or
/// an options object (`{block: 'start'|'center'|'end', behavior}`). The
/// element's document rect is resolved by the input loop from the hit map
/// (the script side has no geometry); `smooth` rides the scroll animation.
pub(crate) fn native_scroll_into_view(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let first = args.first().cloned().unwrap_or_else(JsValue::undefined);
    let (block, smooth) = if first.is_object() {
        let obj = first.as_object().expect("is_object");
        let block = if obj.has_property(js_string!("block"), context)? {
            let keyword = obj
                .get(js_string!("block"), context)?
                .to_string(context)?
                .to_std_string_escaped();
            match keyword.as_str() {
                "center" => ScrollIntoViewBlock::Center,
                "end" => ScrollIntoViewBlock::End,
                _ => ScrollIntoViewBlock::Start,
            }
        } else {
            ScrollIntoViewBlock::Start
        };
        let smooth = if obj.has_property(js_string!("behavior"), context)? {
            let behavior = obj
                .get(js_string!("behavior"), context)?
                .to_string(context)?
                .to_std_string_escaped();
            behavior.eq_ignore_ascii_case("smooth")
        } else {
            false
        };
        (block, smooth)
    } else if first.as_boolean() == Some(false) {
        (ScrollIntoViewBlock::End, false)
    } else {
        (ScrollIntoViewBlock::Start, false)
    };
    if let Some(rc) = super::with_state() {
        rc.borrow_mut().render_cmds.push(RenderCmd::ScrollIntoView {
            node_id,
            block,
            smooth,
        });
    }
    Ok(JsValue::undefined())
}

// ============================================================
// viewport / scroll getters
// ============================================================

/// The shared UI state as `(cols, rows, scroll_y)`; zeros when no input loop
/// is attached (a bare `ScriptEngine::new`).
fn ui() -> (u32, u32, u32) {
    with_ui().unwrap_or((0, 0, 0))
}

/// Clamp an f64 scroll coordinate into the `u32` range (negative values and
/// `NaN` floor to 0; the input loop caps against the live scroll ceiling).
fn clamp_cell(v: f64) -> u32 {
    if v.is_nan() {
        return 0;
    }
    v.clamp(0.0, f64::from(u32::MAX)) as u32
}

pub(crate) fn native_inner_width(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(f64::from(ui().0).into())
}

pub(crate) fn native_inner_height(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(f64::from(ui().1).into())
}

pub(crate) fn native_scroll_y(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(f64::from(ui().2).into())
}

// ============================================================
// scrollTo / scrollBy / scroll
// ============================================================

/// Resolve `(target_y, smooth)` from the arguments of
/// `scrollTo` / `scrollBy`: either the `(x, y)` number form or the
/// `{top, left, behavior}` object form. `relative` adds the current offset
/// (`scrollBy` semantics); a missing `top` keeps the current position (the
/// x axis does not exist in the terminal model and is ignored).
fn scroll_args(
    args: &[JsValue],
    context: &mut Context,
    relative: bool,
) -> JsResult<Option<(u32, bool)>> {
    let (_, _, cur) = ui();
    let current = f64::from(cur);
    let Some(first) = args.first() else {
        return Ok(None);
    };
    if first.is_object() {
        let obj = first.as_object().expect("is_object");
        let smooth = if obj.has_property(js_string!("behavior"), context)? {
            let behavior = obj
                .get(js_string!("behavior"), context)?
                .to_string(context)?
                .to_std_string_escaped();
            behavior.eq_ignore_ascii_case("smooth")
        } else {
            false
        };
        let top = if obj.has_property(js_string!("top"), context)? {
            obj.get(js_string!("top"), context)?.to_number(context)?
        } else {
            current
        };
        let target = if relative { current + top } else { top };
        Ok(Some((clamp_cell(target), smooth)))
    } else if first.is_undefined() {
        Ok(None)
    } else {
        let y = args
            .get(1)
            .unwrap_or(&JsValue::undefined())
            .to_number(context)
            .unwrap_or(current);
        let target = if relative { current + y } else { y };
        Ok(Some((clamp_cell(target), false)))
    }
}

/// Push a page-scroll render command into the pending queue (picked up by
/// the dispatch tail / timer / pending-cmd drains, folded by the input loop).
fn push_scroll_cmd(cmd: RenderCmd) {
    if let Some(rc) = super::with_state() {
        rc.borrow_mut().render_cmds.push(cmd);
    }
}

/// Push the scroll command matching `smooth`: the eased glide marker or the
/// instant `ScrollSet` (the input loop owns both conversions).
fn push_page_scroll(to: u32, smooth: bool) {
    let cmd = if smooth {
        RenderCmd::ScrollAnimate {
            target: ScrollTarget::Page,
            to,
        }
    } else {
        RenderCmd::ScrollSet { to }
    };
    push_scroll_cmd(cmd);
}

/// `window.scrollTo(x, y)` / `window.scrollTo({top, left, behavior})` — move
/// the page viewport. `behavior: "smooth"` glides through the scroll
/// animation; the default is an instant jump. `to` is capped by the input
/// loop's live scroll ceiling.
pub(crate) fn native_scroll_to(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some((to, smooth)) = scroll_args(args, context, false)? {
        push_page_scroll(to, smooth);
    }
    Ok(JsValue::undefined())
}

/// `window.scrollBy(dx, dy)` / `window.scrollBy({top, left, behavior})` —
/// relative variant of [`native_scroll_to`].
pub(crate) fn native_scroll_by(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some((to, smooth)) = scroll_args(args, context, true)? {
        push_page_scroll(to, smooth);
    }
    Ok(JsValue::undefined())
}

// ============================================================
// element geometry reads
// ============================================================

/// The published geometry of one node: the union of every box it laid out,
/// the first box's client rect, and its scrollbar state when the node is an
/// active scroll container. All rects are document (canvas) coordinates in
/// terminal cells.
struct NodeGeometry {
    rect: Rect,
    client: Rect,
    scroll: Option<crate::events::ScrollbarRegion>,
}

/// Union of two rects covering both.
fn union_rects(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width).saturating_sub(x),
        height: (a.y + a.height).max(b.y + b.height).saturating_sub(y),
    }
}

/// Snapshot `node_id`'s geometry from the latest hit map. `None` when no
/// input loop is attached or the node laid out no boxes (unknown / display
/// none / detached).
fn node_geometry(node_id: NodeId) -> Option<NodeGeometry> {
    let hit_map = with_hit_map()?;
    let mut rect: Option<Rect> = None;
    let mut client: Option<Rect> = None;
    for region in hit_map.iter().filter(|r| r.node_id == node_id) {
        rect = Some(match rect {
            Some(prev) => union_rects(prev, region.rect),
            None => region.rect,
        });
        if client.is_none() {
            client = Some(region.client);
        }
    }
    let rect = rect?;
    Some(NodeGeometry {
        rect,
        client: client.unwrap_or(rect),
        scroll: hit_map.scrollbars.get(&node_id).copied(),
    })
}

/// The page scroll offset (viewport y = document y − scroll offset).
fn scroll_y() -> u32 {
    with_ui().map_or(0, |(_, _, sy)| sy)
}

/// `element.getBoundingClientRect()` — the union of the element's laid-out
/// boxes as `{x, y, width, height, top, right, bottom, left}` in terminal
/// cells. `y`/`top` are viewport coordinates (document minus page scroll);
/// boxes scrolled above the viewport read as 0 (unsigned cell space).
pub(crate) fn native_get_bounding_client_rect(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let mut init = ObjectInitializer::new(context);
    if let Some(geo) = node_geometry(node_id) {
        let top = geo.rect.y.saturating_sub(scroll_y() as usize);
        let x = f64::from(geo.rect.x as u32);
        let y = f64::from(top as u32);
        let w = f64::from(geo.rect.width as u32);
        let h = f64::from(geo.rect.height as u32);
        init.property(js_string!("x"), x, Attribute::all());
        init.property(js_string!("y"), y, Attribute::all());
        init.property(js_string!("width"), w, Attribute::all());
        init.property(js_string!("height"), h, Attribute::all());
        init.property(js_string!("top"), y, Attribute::all());
        init.property(js_string!("left"), x, Attribute::all());
        init.property(js_string!("right"), x + w, Attribute::all());
        init.property(js_string!("bottom"), y + h, Attribute::all());
    } else {
        for name in [
            "x", "y", "width", "height", "top", "right", "bottom", "left",
        ] {
            init.property(js_string!(name), 0.0, Attribute::all());
        }
    }
    Ok(init.build().into())
}

/// Shared numeric getter over the node's client box (`clientWidth` /
/// `clientHeight`) or its border inset (`clientTop` / `clientLeft`).
fn client_metric(node_id: NodeId, pick: fn(&NodeGeometry) -> f64) -> f64 {
    node_geometry(node_id).map_or(0.0, |g| pick(&g))
}

pub(crate) fn native_get_client_width(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    Ok(client_metric(node_id, |g| f64::from(g.client.width as u32)).into())
}

pub(crate) fn native_get_client_height(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    Ok(client_metric(node_id, |g| f64::from(g.client.height as u32)).into())
}

pub(crate) fn native_get_client_top(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    Ok(client_metric(node_id, |g| {
        f64::from(g.client.y.saturating_sub(g.rect.y) as u32)
    })
    .into())
}

pub(crate) fn native_get_client_left(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    Ok(client_metric(node_id, |g| {
        f64::from(g.client.x.saturating_sub(g.rect.x) as u32)
    })
    .into())
}

pub(crate) fn native_get_scroll_top(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    Ok(f64::from(
        node_geometry(node_id)
            .and_then(|g| g.scroll)
            .map_or(0, |s| s.scroll_y),
    )
    .into())
}

/// `element.scrollTop = n` — re-scroll an active native scroll container
/// (clamped to its live maximum); a no-op on non-scrolling elements.
pub(crate) fn native_set_scroll_top(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args.first().cloned().unwrap_or_else(JsValue::undefined);
    let to = clamp_cell(value.to_number(context).unwrap_or(0.0));
    if let Some(geo) = node_geometry(node_id)
        && let Some(scroll) = geo.scroll
    {
        push_scroll_cmd(RenderCmd::RenderScroll {
            node_id,
            scroll_y: to.min(scroll.max_scroll),
        });
    }
    Ok(JsValue::undefined())
}

pub(crate) fn native_get_scroll_left(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let _ = this_node_id(this, context)?;
    // Horizontal container scrolling is not implemented yet (the x axis is
    // read-only zero until then, like `window.scrollX`).
    Ok(0.0.into())
}

pub(crate) fn native_get_scroll_height(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = node_geometry(node_id).map_or(0.0, |g| {
        f64::from(
            g.scroll
                .map_or(g.rect.height, |s| s.content_height as usize) as u32,
        )
    });
    Ok(value.into())
}

pub(crate) fn native_get_scroll_width(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    Ok(node_geometry(node_id)
        .map_or(0.0, |g| f64::from(g.rect.width as u32))
        .into())
}

/// `document.activeElement` — the focused `<input>` / `<textarea>` wrapper,
/// or `null`. Reads the input loop's shared focus snapshot; before the first
/// focus it is `null`.
pub(crate) fn native_get_active_element(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let bits = with_focused();
    if bits == 0 {
        return Ok(JsValue::null());
    }
    super::node_wrapper(context, super::u64_to_node_id(bits))
}

// ============================================================
// selection
// ============================================================

/// `window.getSelection()` / `document.getSelection()` — a minimal Selection
/// snapshot over the engine's page selection: `isCollapsed`, `rangeCount`,
/// `anchorNode` / `focusNode` (always `null` — the engine selection is
/// canvas-space, not node-anchored) and `toString()`.
pub(crate) fn native_get_selection(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let text = selection_text();
    let collapsed = text.is_empty();
    let mut init = ObjectInitializer::new(context);
    init.property(
        js_string!("rangeCount"),
        if collapsed { 0 } else { 1 },
        Attribute::all(),
    );
    init.property(js_string!("isCollapsed"), collapsed, Attribute::all());
    init.property(js_string!("anchorNode"), JsValue::null(), Attribute::all());
    init.property(js_string!("focusNode"), JsValue::null(), Attribute::all());
    let obj = init.build();
    obj.set(
        js_string!("toString"),
        js_fn(
            context,
            NativeFunction::from_fn_ptr(native_selection_to_string),
        ),
        false,
        context,
    )?;
    Ok(obj.into())
}

/// `Selection.toString()` — the selected text snapshot, read fresh on every
/// call.
pub(crate) fn native_selection_to_string(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(js_string!(selection_text()).into())
}

// ============================================================
// matchMedia
// ============================================================

/// Evaluate a media query string against the current viewport. An empty or
/// unparseable query matches nothing (the CSS unknown-query rule).
fn matches_now(query: &str) -> bool {
    let (cols, rows, _) = ui();
    let viewport = Viewport {
        cols: cols as usize,
        rows: rows as usize,
    };
    parse_media_query_list(query).is_none_or(|list| list.matches(&viewport))
}

/// `matches` getter: re-evaluated on every access, so a held `MQL` object
/// always reflects the current viewport (live, like the browser's).
fn mql_matches_getter(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let obj = this.to_object(context)?;
    let query = obj
        .get(js_string!("_query"), context)?
        .to_string(context)?
        .to_std_string_escaped();
    Ok(matches_now(&query).into())
}

/// `MQL.addEventListener(type, fn)` — only the `"change"` type carries
/// meaning; the listener list lives on the object itself (JS-side GC root).
fn mql_add_listener(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let type_name = args
        .first()
        .cloned()
        .unwrap_or_else(JsValue::undefined)
        .to_string(context)?
        .to_std_string_escaped();
    let listener = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if type_name != "change" {
        return Ok(JsValue::undefined());
    }
    if !listener.is_callable() {
        return Err(JsNativeError::typ()
            .with_message("addEventListener expects a function")
            .into());
    }
    let obj = this.to_object(context)?;
    let list = obj
        .get(js_string!("_listeners"), context)?
        .to_object(context)?;
    let len = list.get(js_string!("length"), context)?.to_u32(context)?;
    // EventTarget dedup: registering the same function twice keeps one entry.
    for i in 0..len {
        if list.get(i, context)?.strict_equals(&listener) {
            return Ok(JsValue::undefined());
        }
    }
    list.set(len, listener, false, context)?;
    Ok(JsValue::undefined())
}

/// `MQL.removeEventListener(type, fn)` — identity match against the list.
fn mql_remove_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let type_name = args
        .first()
        .cloned()
        .unwrap_or_else(JsValue::undefined)
        .to_string(context)?
        .to_std_string_escaped();
    if type_name != "change" {
        return Ok(JsValue::undefined());
    }
    let listener = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    let obj = this.to_object(context)?;
    let list = obj
        .get(js_string!("_listeners"), context)?
        .to_object(context)?;
    let len = list.get(js_string!("length"), context)?.to_u32(context)?;
    let mut kept = Vec::new();
    for i in 0..len {
        let item = list.get(i, context)?;
        if !item.strict_equals(&listener) {
            kept.push(item);
        }
    }
    let new_list: JsValue = {
        let mut init = ObjectInitializer::new(context);
        for (i, v) in kept.iter().enumerate() {
            init.property(
                i,
                v.clone(),
                Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
            );
        }
        init.build().into()
    };
    obj.set(js_string!("_listeners"), new_list, false, context)?;
    Ok(JsValue::undefined())
}

/// `window.matchMedia(query)` — build a `MediaQueryList` object with a live
/// `matches` getter and a `change` listener slot, registered in the
/// `__htuiMQLs` array so a resize can re-evaluate it (firing `change` on
/// flips). Parsing reuses the CSS `@media` grammar, so JS and stylesheet
/// queries agree by construction.
pub(crate) fn native_match_media(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let query = args
        .first()
        .cloned()
        .unwrap_or_else(JsValue::undefined)
        .to_string(context)?
        .to_std_string_escaped();
    let empty_listeners = JsValue::from(JsArray::new(context));
    let mut init = ObjectInitializer::new(context);
    let obj = init
        .property(
            js_string!("media"),
            js_string!(query.clone()),
            Attribute::all(),
        )
        .property(
            js_string!("_query"),
            js_string!(query.clone()),
            Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
        )
        .property(
            js_string!("_lastMatches"),
            matches_now(&query),
            Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
        )
        .property(
            js_string!("_listeners"),
            empty_listeners,
            Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
        )
        .build();
    let desc = PropertyDescriptorBuilder::new()
        .get(js_fn(
            context,
            NativeFunction::from_fn_ptr(mql_matches_getter),
        ))
        .enumerable(true)
        .configurable(true)
        .build();
    obj.define_property_or_throw(js_string!("matches"), desc, context)?;
    obj.set(
        js_string!("addEventListener"),
        js_fn(context, NativeFunction::from_fn_ptr(mql_add_listener)),
        false,
        context,
    )?;
    obj.set(
        js_string!("removeEventListener"),
        js_fn(context, NativeFunction::from_fn_ptr(mql_remove_listener)),
        false,
        context,
    )?;
    // GC-root the MQL in the bootstrap array so `change` can be fired later.
    let global = context.global_object();
    let mqls = global
        .get(js_string!("__htuiMQLs"), context)?
        .to_object(context)?;
    let len = mqls.get(js_string!("length"), context)?.to_u32(context)?;
    mqls.set(len, obj.clone(), false, context)?;
    Ok(obj.into())
}

// ============================================================
// window.addEventListener / removeEventListener
// ============================================================

/// `window.addEventListener(type, fn)` — registered under the `"window"`
/// registry key (the global object has no node identity).
pub(crate) fn native_add_window_event_listener(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let type_name = args
        .first()
        .cloned()
        .unwrap_or_else(JsValue::undefined)
        .to_string(context)?
        .to_std_string_escaped();
    let listener = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !listener.is_callable() {
        return Err(JsNativeError::typ()
            .with_message("addEventListener expects a function")
            .into());
    }
    if type_name == "wheel" {
        crate::script::note_wheel_listener();
    }
    let registry = window_listeners_registry(context)?;
    let entry = match registry
        .get(js_string!(WINDOW_KEY), context)?
        .to_object(context)
    {
        Ok(entry) => entry,
        Err(_) => {
            let fresh = ObjectInitializer::new(context).build();
            registry.set(js_string!(WINDOW_KEY), fresh.clone(), false, context)?;
            fresh
        }
    };
    let arr = match entry
        .get(js_string!(type_name.clone()), context)?
        .to_object(context)
    {
        Ok(arr) => arr,
        Err(_) => {
            let fresh = JsValue::from(JsArray::new(context));
            entry.set(js_string!(type_name), fresh.clone(), false, context)?;
            fresh.to_object(context)?
        }
    };
    let len = arr.get(js_string!("length"), context)?.to_u32(context)?;
    for i in 0..len {
        if arr.get(i, context)?.strict_equals(&listener) {
            return Ok(JsValue::undefined());
        }
    }
    arr.set(len, listener, false, context)?;
    Ok(JsValue::undefined())
}

/// `window.removeEventListener(type, fn)` — identity match, like the element
/// form.
pub(crate) fn native_remove_window_event_listener(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let type_name = args
        .first()
        .cloned()
        .unwrap_or_else(JsValue::undefined)
        .to_string(context)?
        .to_std_string_escaped();
    let listener = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    let registry = window_listeners_registry(context)?;
    let Ok(entry) = registry
        .get(js_string!(WINDOW_KEY), context)?
        .to_object(context)
    else {
        return Ok(JsValue::undefined());
    };
    let Ok(arr) = entry
        .get(js_string!(type_name.clone()), context)?
        .to_object(context)
    else {
        return Ok(JsValue::undefined());
    };
    let len = arr.get(js_string!("length"), context)?.to_u32(context)?;
    let mut kept = Vec::new();
    for i in 0..len {
        let item = arr.get(i, context)?;
        if !item.strict_equals(&listener) {
            kept.push(item);
        }
    }
    let new_arr: JsValue = {
        let mut init = ObjectInitializer::new(context);
        for (i, v) in kept.iter().enumerate() {
            init.property(
                i,
                v.clone(),
                Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
            );
        }
        init.build().into()
    };
    entry.set(js_string!(type_name), new_arr, false, context)?;
    Ok(JsValue::undefined())
}

/// The `__htuiListeners` registry object (created empty by `bootstrap.js`).
fn window_listeners_registry(context: &mut Context) -> JsResult<boa_engine::JsObject> {
    let global = context.global_object();
    global
        .get(js_string!("__htuiListeners"), context)?
        .to_object(context)
}

/// Window-level listeners for `type_name` (may be empty).
pub(crate) fn get_window_listeners(
    context: &mut Context,
    type_name: &str,
) -> JsResult<Vec<JsValue>> {
    let registry = window_listeners_registry(context)?;
    let entry = registry.get(js_string!(WINDOW_KEY), context)?;
    if entry.is_undefined() {
        return Ok(Vec::new());
    }
    let entry = entry.to_object(context)?;
    let arr = entry.get(js_string!(type_name), context)?;
    if arr.is_undefined() {
        return Ok(Vec::new());
    }
    let arr = arr.to_object(context)?;
    let len = arr.get(js_string!("length"), context)?.to_u32(context)?;
    let mut out = Vec::with_capacity(len.min(1024) as usize);
    for i in 0..len {
        out.push(arr.get(i, context)?);
    }
    Ok(out)
}

/// Read the parsed query of one registered `matchMedia` object.
pub(crate) fn mql_query(obj: &boa_engine::JsObject, context: &mut Context) -> Option<String> {
    obj.get(js_string!("_query"), context)
        .ok()
        .and_then(|v| v.to_string(context).ok())
        .map(|s| s.to_std_string_escaped())
}

/// The `_lastMatches` flag of one registered `matchMedia` object.
pub(crate) fn mql_last_matches(obj: &boa_engine::JsObject, context: &mut Context) -> bool {
    obj.get(js_string!("_lastMatches"), context)
        .ok()
        .is_some_and(|v| v.to_boolean())
}

/// Whether a `MediaQueryList` parses to a query list matching the current
/// viewport (the shared evaluation behind `matches` and the `change` sweep).
pub(crate) fn mql_matches_now(query: &str) -> bool {
    matches_now(query)
}
