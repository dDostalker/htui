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
//! - `fixed` — the viewport.
//!
//! Known v1 limitation: `fixed` boxes are anchored to the viewport top-left
//! (cell 0,0) but still scroll with the document (no scroll-aware anchoring).

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
        Some(l) => cb.x.saturating_add(l),
        None => match inset.right {
            Some(r) => cb
                .x
                .saturating_add(cb.width)
                .saturating_sub(b.rect.width)
                .saturating_sub(r),
            None => cb.x,
        },
    };
    let new_y = match inset.top {
        Some(t) => cb.y.saturating_add(t),
        None => match inset.bottom {
            Some(bot) => cb
                .y
                .saturating_add(cb.height)
                .saturating_sub(b.rect.height)
                .saturating_sub(bot),
            None => cb.y,
        },
    };
    let dx = new_x as i64 - b.rect.x as i64;
    let dy = new_y as i64 - b.rect.y as i64;
    translate_subtree(b, dx, dy);
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
