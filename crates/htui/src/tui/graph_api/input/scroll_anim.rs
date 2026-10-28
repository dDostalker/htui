//! Eased scroll animation for the wheel and page-jump keys.
//!
//! Trigger points (the wheel arms in `refresh` / `mouse`, the scroll keys in
//! `page`) do not move the viewport themselves when smooth scrolling is
//! enabled — they push a [`RenderCmd::ScrollAnimate`](super::RenderCmd::ScrollAnimate)
//! marker naming the target offset. The run loop converts markers into a
//! [`ScrollAnim`] on the input state and advances it once per iteration,
//! emitting one viewport command per frame. Positions derive from the wall
//! clock, so a late wake self-corrects instead of accumulating lag.

use std::time::{Duration, Instant};

use crate::tdom::node::NodeId;

use super::InputState;

/// What a scroll animation moves: the page viewport or one native scroll
/// container (identified by its session-local node id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    /// The page viewport ([`InputState::scroll_y`]).
    Page,
    /// A native scroll container ([`InputState::scroll_offsets`]).
    Container(NodeId),
}

/// The `block` alignment of JS `element.scrollIntoView(arg)`: where the
/// element lands inside the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollIntoViewBlock {
    /// Align the element's top with the viewport top (also `true`'s meaning).
    Start,
    /// Center the element in the viewport.
    Center,
    /// Align the element's bottom with the viewport bottom (also `false`'s
    /// meaning).
    End,
}

/// The page scroll offset that puts `rect` (a document-space element rect)
/// at `block` alignment inside a `viewport_rows`-tall viewport.
pub fn scroll_into_view_target(
    rect: crate::layout::Rect,
    block: ScrollIntoViewBlock,
    viewport_rows: u32,
) -> u32 {
    let (y, height) = (
        u32::try_from(rect.y).unwrap_or(u32::MAX),
        u32::try_from(rect.height).unwrap_or(u32::MAX),
    );
    match block {
        ScrollIntoViewBlock::Start => y,
        ScrollIntoViewBlock::Center => y.saturating_sub(viewport_rows.saturating_sub(height) / 2),
        ScrollIntoViewBlock::End => y.saturating_add(height).saturating_sub(viewport_rows),
    }
}

/// Input-loop poll cadence while a scroll animation is active (~60 fps, the
/// same beat the script layer's `requestAnimationFrame` runs at).
pub const ANIM_POLL_MS: u64 = 16;

/// Shortest animation — a single wheel tick must still read as motion.
const MIN_DURATION_MS: u64 = 80;
/// Longest animation — document jumps must stay snappy.
const MAX_DURATION_MS: u64 = 280;
/// Animation length per row of travel between the start and target offsets.
const MS_PER_ROW: u64 = 3;

/// Wall-clock duration for animating from `from` to `to`: proportional to the
/// travel distance, clamped to a perceptible minimum and a non-lingering
/// maximum.
pub fn scroll_anim_duration(from: u32, to: u32) -> Duration {
    let rows = u64::from(from.abs_diff(to));
    Duration::from_millis((rows * MS_PER_ROW).clamp(MIN_DURATION_MS, MAX_DURATION_MS))
}

/// Ease-out cubic: fast start, decelerating stop (the browser smooth-scroll
/// feel). `t` is clamped to `0.0..=1.0`.
pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The offset a wheel / page step lands on, or `None` when the step is a
/// no-op. `base` is the active animation's target when one is already running
/// for the scroll axis (so repeated input extends the glide instead of
/// jumping), and a step that reaches the offset the axis already heads for is
/// dropped — the same boundary semantics the instant scroll path uses.
pub fn step_target(base: u32, delta: i32, min: u32, max: u32) -> Option<u32> {
    let next = if delta < 0 {
        base.saturating_sub(delta.unsigned_abs())
    } else {
        base.saturating_add(u32::try_from(delta).unwrap_or(u32::MAX))
    };
    let next = next.clamp(min, max);
    (next != base).then_some(next)
}

/// Target offset of the animation running for `kind`, if any — the base that
/// future trigger steps build on.
pub fn active_scroll_target(state: &InputState, kind: ScrollTarget) -> Option<u32> {
    match &state.scroll_anim {
        Some(anim) if anim.target() == kind => Some(anim.to()),
        _ => None,
    }
}

/// One eased scroll animation (a single tween from a start offset to a target
/// offset).
///
/// Lives on the input loop ([`InputState::scroll_anim`]); only one runs at a
/// time — input for another scroll axis replaces it, input for the same axis
/// retargets it.
#[derive(Debug)]
pub struct ScrollAnim {
    target: ScrollTarget,
    from: u32,
    to: u32,
    start: Instant,
    duration: Duration,
}

impl ScrollAnim {
    /// Start a tween from `from` to `to` at instant `now`.
    pub fn start(target: ScrollTarget, from: u32, to: u32, now: Instant) -> Self {
        Self {
            target,
            from,
            to,
            start: now,
            duration: scroll_anim_duration(from, to),
        }
    }

    /// The scroll axis this animation moves.
    pub fn target(&self) -> ScrollTarget {
        self.target
    }

    /// The offset the animation settles on.
    pub fn to(&self) -> u32 {
        self.to
    }

    /// The eased position at instant `now` (exactly `to` once finished).
    pub fn position(&self, now: Instant) -> u32 {
        let t =
            now.saturating_duration_since(self.start).as_secs_f64() / self.duration.as_secs_f64();
        let eased = ease_out_cubic(t);
        (f64::from(self.from) + f64::from(self.to - self.from) * eased).round() as u32
    }

    /// Whether the tween has run its full duration (the position is then
    /// exactly `to`).
    pub fn finished(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }

    /// Redirect the glide to `to` from wherever the animation currently is:
    /// repeated input extends the motion without a positional jump.
    pub fn retarget(&mut self, to: u32, now: Instant) {
        self.from = self.position(now);
        self.to = to;
        self.start = now;
        self.duration = scroll_anim_duration(self.from, to);
    }
}
