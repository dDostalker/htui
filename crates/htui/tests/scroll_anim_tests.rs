//! Scroll-animation tests — the pure easing math behind the input loop's
//! smooth scrolling (wheel + page-jump glide). Purely in-memory: instants are
//! injected into the tween, never slept on.

use std::time::{Duration, Instant};

use htui::tui::graph_api::{
    InputState, ScrollAnim, ScrollTarget, active_scroll_target, ease_out_cubic,
    scroll_anim_duration, step_target,
};

#[test]
fn ease_out_cubic_endpoints_clamp_and_monotonicity() {
    assert_eq!(ease_out_cubic(0.0), 0.0);
    assert_eq!(ease_out_cubic(1.0), 1.0);
    assert_eq!(ease_out_cubic(-0.5), 0.0, "clamped below the domain");
    assert_eq!(ease_out_cubic(1.5), 1.0, "clamped above the domain");
    let mut prev = 0.0;
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        let v = ease_out_cubic(t);
        assert!(v >= prev, "ease must be monotonic non-decreasing");
        prev = v;
    }
}

#[test]
fn duration_scales_with_distance_and_clamps() {
    assert_eq!(
        scroll_anim_duration(0, 3),
        Duration::from_millis(80),
        "a wheel tick rides the minimum"
    );
    assert_eq!(
        scroll_anim_duration(5, 5),
        Duration::from_millis(80),
        "zero travel keeps the minimum"
    );
    assert_eq!(
        scroll_anim_duration(0, 40),
        Duration::from_millis(120),
        "40 rows * 3ms per row (a page step)"
    );
    assert_eq!(
        scroll_anim_duration(0, 500),
        Duration::from_millis(280),
        "long jumps ride the maximum"
    );
    assert_eq!(
        scroll_anim_duration(500, 0),
        scroll_anim_duration(0, 500),
        "direction does not matter"
    );
}

#[test]
fn position_hits_both_endpoints_exactly() {
    let start = Instant::now();
    let anim = ScrollAnim::start(ScrollTarget::Page, 100, 500, start);
    assert_eq!(anim.position(start), 100, "starts at `from`");
    // 400 rows * 3ms = 1200ms, clamped to the 280ms maximum.
    assert_eq!(
        anim.position(start + Duration::from_millis(280)),
        500,
        "ends exactly at `to`"
    );
    assert_eq!(
        anim.position(start + Duration::from_millis(10_000)),
        500,
        "clamped past the end"
    );
    assert!(anim.finished(start + Duration::from_millis(280)));
    assert!(!anim.finished(start + Duration::from_millis(279)));
}

#[test]
fn position_is_monotonic_and_front_loaded() {
    let start = Instant::now();
    // 300 rows * 3ms = 900ms, clamped to the 280ms maximum.
    let anim = ScrollAnim::start(ScrollTarget::Page, 0, 300, start);
    let mut prev = 0;
    for ms in [0u64, 20, 40, 70, 100, 140, 190, 240, 280] {
        let p = anim.position(start + Duration::from_millis(ms));
        assert!(p >= prev, "position must be monotonic non-decreasing");
        prev = p;
    }
    assert_eq!(prev, 300);
    // Ease-out front-loads the motion: half the time covers most of the
    // distance (t = 0.5 → eased 0.875).
    assert!(
        anim.position(start + Duration::from_millis(140)) > 150,
        "ease-out must move faster at the start"
    );
}

#[test]
fn retarget_continues_from_the_current_position() {
    let start = Instant::now();
    let mut anim = ScrollAnim::start(ScrollTarget::Page, 0, 300, start);
    let mid = start + Duration::from_millis(140);
    let current = anim.position(mid);
    anim.retarget(400, mid);
    assert_eq!(
        anim.position(mid),
        current,
        "retarget must not jump the position"
    );
    assert_eq!(anim.to(), 400);
    assert!(!anim.finished(mid), "retarget restarts the clock");
    assert_eq!(anim.position(mid + Duration::from_millis(280)), 400);
}

#[test]
fn step_target_clamps_and_drops_noops() {
    assert_eq!(step_target(10, 3, 0, 100), Some(13));
    assert_eq!(step_target(10, -3, 0, 100), Some(7));
    assert_eq!(step_target(0, -3, 0, 100), None, "at the top boundary");
    assert_eq!(step_target(100, 3, 0, 100), None, "at the bottom boundary");
    assert_eq!(
        step_target(99, 3, 0, 100),
        Some(100),
        "clamped to the ceiling"
    );
    assert_eq!(step_target(2, -3, 0, 100), Some(0), "clamped to the floor");
    assert_eq!(
        step_target(0, i32::MIN, 0, 100),
        None,
        "extreme negative delta saturates into a no-op, not a panic"
    );
}

#[test]
fn active_scroll_target_reports_only_the_running_axis() {
    let mut state = InputState::new();
    assert_eq!(
        active_scroll_target(&state, ScrollTarget::Page),
        None,
        "no animation running"
    );
    let now = Instant::now();
    state.scroll_anim = Some(ScrollAnim::start(ScrollTarget::Page, 0, 90, now));
    assert_eq!(active_scroll_target(&state, ScrollTarget::Page), Some(90));
}
