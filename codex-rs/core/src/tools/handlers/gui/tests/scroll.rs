use super::*;

#[test]
fn normalize_scroll_direction_defaults_and_validates() {
    assert!(matches!(
        normalize_scroll_direction(None).unwrap(),
        ScrollDirection::Down
    ));
    assert!(matches!(
        normalize_scroll_direction(Some("left")).unwrap(),
        ScrollDirection::Left
    ));
    assert!(normalize_scroll_direction(Some("sideways")).is_err());
}

#[test]
fn resolve_scroll_plan_uses_semantic_distance_defaults() {
    let capture_bounds = HelperRect {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 800.0,
    };

    let targetless = resolve_scroll_plan(
        None,
        None,
        false,
        ScrollDirection::Down,
        None,
        Some(&capture_bounds),
    );
    assert_eq!(targetless.distance_preset, "page");
    assert_eq!(targetless.unit, "pixel");
    assert_eq!(targetless.amount, 600);

    let target_bounds = HelperRect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 200.0,
    };
    let targeted = resolve_scroll_plan(
        None,
        None,
        true,
        ScrollDirection::Down,
        Some(&target_bounds),
        Some(&capture_bounds),
    );
    assert_eq!(targeted.distance_preset, "medium");
    assert_eq!(targeted.unit, "pixel");
    assert_eq!(targeted.amount, 100);
}

#[test]
fn scroll_delta_components_match_understudy_scroll_direction_convention() {
    assert_eq!(
        scroll_delta_components(ScrollDirection::Down, 240),
        (0, -240)
    );
    assert_eq!(scroll_delta_components(ScrollDirection::Up, 240), (0, 240));
    assert_eq!(
        scroll_delta_components(ScrollDirection::Left, 240),
        (-240, 0)
    );
    assert_eq!(
        scroll_delta_components(ScrollDirection::Right, 240),
        (240, 0)
    );
}
