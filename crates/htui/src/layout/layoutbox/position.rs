//! Post-layout positioning pass (CSS Positioned Layout).
//!
//! Positioned boxes never participate in their parent's flow:
//!
//! - `position: relative` keeps its normal-flow slot and is then offset in
//!   place — handled during the geometry dispatch, before the tree is
//!   assembled.
//! - `position: absolute` / `fixed` are laid out against a detached local
//!   cursor by the geometry dispatcher and collected as ordinary children
//!   (they occupy no flow rows). This pass runs once over the finished tree,
//!   resolves each box's final rectangle against its containing block, and
//!   translates the whole subtree so descendants keep their relative
//!   positions.
//!
//! Containing-block semantics (v1 subset):
//!
//! - `absolute` — the nearest positioned ancestor's padding-box, or the
//!   initial containing block (the viewport) when none exists.
//! - `fixed` — the viewport, scroll-aware: the layout anchors fixed boxes to
//!   the top-of-document viewport, and [`pin_fixed_boxes`] re-anchors them to
//!   the *current* scroll offset whenever it changes, so a fixed element stays
//!   pinned to its screen position while the document scrolls.

use crate::html::css_parse::{InsetSides, Position};
use crate::layout::{LayoutBox, Rect};

/// Resolve the final rectangles of every positioned box in the forest.
///
/// `viewport` is the terminal viewport in cells: `fixed` boxes anchor to it,
/// and boxes without a positioned ancestor use it as the initial containing
/// block. Called once by the layout-tree orchestrator after the whole forest
/// has been assembled.
pub fn resolve_positioned_rects(boxes: &mut [LayoutBox], viewport: Rect) {
    for b in boxes {
        resolve_positioned_box(b, None, viewport);
    }
}

/// Re-anchor every `position: fixed` box to the viewport at the *current*
/// scroll offset, in document coordinates.
///
/// Fixed boxes are laid out against the top-of-document viewport; scrolling
/// re-slices the canvas without re-laying-out, so the render thread calls
/// this whenever `scroll_y` changes (only when the document contains fixed
/// boxes). It moves fixed boxes by the scroll delta so the canvas slice shows
/// them at their constant screen position; unanchored `absolute` boxes are
/// untouched (they anchor to the document, not the viewport).
pub fn pin_fixed_boxes(boxes: &mut [LayoutBox], viewport: Rect) {
    for b in boxes {
        if matches!(b.positioning.position, Position::Fixed) {
            reposition(b, viewport);
        }
        pin_fixed_boxes(&mut b.children, viewport);
    }
}

/// Re-anchor every `position: sticky` box relative to the *current* scroll
/// offset, in document coordinates.
///
/// Sticky boxes keep their normal-flow slot at layout time; this pass clamps
/// their rectangle into the viewport as the document scrolls (the v1
/// constraint rectangle is the viewport itself, matching the engine's
/// whole-document scroll model). The caller re-runs it from a fresh layout
/// tree whenever `scroll_y` changes, exactly like [`pin_fixed_boxes`].
/// `top` clamps the box to the viewport top once its flow position scrolls
/// past the offset; `bottom` clamps it to the viewport bottom instead.
pub fn pin_sticky_boxes(boxes: &mut [LayoutBox], viewport: Rect) {
    for b in boxes {
        if matches!(b.positioning.position, Position::Sticky) {
            stick(b, viewport);
        }
        pin_sticky_boxes(&mut b.children, viewport);
    }
}

/// Offset a `position: relative` box by its insets.
///
/// The box keeps its flow slot; only its painted position shifts (and with
/// it, the whole subtree). `top` / `left` move down / right, `bottom` /
/// `right` move up / left — mirroring CSS. Called once by the geometry
/// dispatcher, before the positioning pass.
pub fn apply_relative_offset(b: &mut LayoutBox, inset: InsetSides) {
    let dx = match inset.left {
        Some(l) => l as i64,
        None => match inset.right {
            Some(r) => -(r as i64),
            None => 0,
        },
    };
    let dy = match inset.top {
        Some(t) => t as i64,
        None => match inset.bottom {
            Some(bot) => -(bot as i64),
            None => 0,
        },
    };
    if dx != 0 || dy != 0 {
        translate_subtree(b, dx, dy);
    }
}

fn resolve_positioned_box(b: &mut LayoutBox, pos_ancestor: Option<Rect>, viewport: Rect) {
    match b.positioning.position {
        Position::Static => {
            for child in &mut b.children {
                resolve_positioned_box(child, pos_ancestor, viewport);
            }
        }
        Position::Relative => {
            // Already offset in place; it is a positioned ancestor for its
            // descendants.
            for child in &mut b.children {
                resolve_positioned_box(child, Some(b.rect), viewport);
            }
        }
        Position::Absolute => {
            let cb = pos_ancestor.unwrap_or(viewport);
            reposition(b, cb);
            for child in &mut b.children {
                resolve_positioned_box(child, Some(b.rect), viewport);
            }
        }
        Position::Fixed => {
            reposition(b, viewport);
            for child in &mut b.children {
                resolve_positioned_box(child, Some(b.rect), viewport);
            }
        }
        Position::Sticky => {
            // Sticky keeps its flow slot (no inline offset) and is a
            // positioned ancestor for its descendants; the scroll clamp is
            // applied by [`pin_sticky_boxes`].
            for child in &mut b.children {
                resolve_positioned_box(child, Some(b.rect), viewport);
            }
        }
    }
}

/// Move `b` inside containing block `cb` per its insets, then translate the
/// subtree by the delta so descendants keep their relative offsets.
///
/// `left` wins over `right`, `top` over `bottom` (CSS semantics). An
/// unset side leaves the box at the containing block's origin on that axis.
fn reposition(b: &mut LayoutBox, cb: Rect) {
    let inset = b.positioning.inset;
    let new_x = match inset.left {
        Some(l) => (cb.x as i64 + l as i64).max(0) as usize,
        None => match inset.right {
            Some(r) => {
                (cb.x as i64 + cb.width as i64 - b.rect.width as i64 - r as i64).max(0) as usize
            }
            None => cb.x,
        },
    };
    let new_y = match inset.top {
        Some(t) => (cb.y as i64 + t as i64).max(0) as usize,
        None => match inset.bottom {
            Some(boot) => (cb.y as i64 + cb.height as i64 - b.rect.height as i64 - boot as i64)
                .max(0) as usize,
            None => cb.y,
        },
    };
    let dx = new_x as i64 - b.rect.x as i64;
    let dy = new_y as i64 - b.rect.y as i64;
    translate_subtree(b, dx, dy);
}

/// Clamp a sticky box's vertical position into the viewport.
///
/// The box's rect may already be pinned to a previous scroll offset (the
/// geometry cache keeps pinned boxes across scrolls), so the **normal-flow**
/// row is captured on the first pin — when the rect still carries the flow
/// position — and reused afterwards. `top` (or `bottom`, when set) is the gap
/// kept between the box and the viewport edge. The horizontal axis is
/// untouched (the engine scrolls vertically only).
fn stick(b: &mut LayoutBox, viewport: Rect) {
    if b.sticky_flow_y.is_none() {
        b.sticky_flow_y = Some(b.rect.y);
    }
    let flow_y = b.sticky_flow_y.unwrap_or(b.rect.y);
    let h = b.rect.height;
    let top_inset = b.positioning.inset.top.unwrap_or(0);
    let bottom_inset = b.positioning.inset.bottom.unwrap_or(0);
    let min_y = (viewport.y as i64 + top_inset as i64).max(0) as usize;
    let max_y = (viewport.y as i64 + viewport.height as i64 - bottom_inset as i64 - h as i64).max(0)
        as usize;
    let target_y = if max_y < min_y {
        // The box is taller than the available viewport — anchor at the top.
        min_y
    } else if flow_y < min_y {
        min_y
    } else if flow_y > max_y {
        max_y
    } else {
        flow_y
    };
    // Translate from the *current* rect (which may already be pinned to a
    // previous scroll) to the target — the recorded flow row is only the
    // clamp input, not the box's current position.
    let dy = target_y as i64 - b.rect.y as i64;
    if dy != 0 {
        translate_subtree(b, 0, dy);
    }
}

/// Shift a box's rect (and every descendant's) by `dx` / `dy`.
fn translate_subtree(b: &mut LayoutBox, dx: i64, dy: i64) {
    b.rect.x = shift(b.rect.x, dx);
    b.rect.y = shift(b.rect.y, dy);
    for child in &mut b.children {
        translate_subtree(child, dx, dy);
    }
}

/// Clamp a `usize` coordinate after a signed shift (never below 0).
fn shift(v: usize, d: i64) -> usize {
    if d >= 0 {
        v.saturating_add(d as usize)
    } else {
        v.saturating_sub(d.unsigned_abs() as usize)
    }
}
