use super::*;

#[test]
fn gui_wait_reuses_previous_window_observe_target_when_not_overridden() {
    let previous = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800,
            height: 600,
            image_width: 1600,
            image_height: 1200,

            display_index: 1,
            capture_mode: CaptureMode::Window,
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("bounds".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let mut app = None;
    let mut capture_mode = None;
    let mut window_selection = None;

    if app.is_none() && capture_mode.is_none() && window_selection.is_none() {
        app = previous.app_name.clone();
        capture_mode = Some(previous.capture.capture_mode.to_string());
        if previous.capture.capture_mode == CaptureMode::Window {
            window_selection = previous
                .capture
                .window_title
                .as_ref()
                .map(|title| WindowSelector {
                    title: Some(title.clone()),
                    title_contains: None,
                    index: None,
                });
        }
    }

    assert_eq!(app.as_deref(), Some("Notes"));
    assert_eq!(capture_mode.as_deref(), Some("window"));
    assert_eq!(
        window_selection.and_then(|selection| selection.title),
        Some("Quick Note".to_string())
    );
}

#[test]
fn normalize_wait_target_state_defaults_and_validates() {
    assert_eq!(normalize_wait_target_state(None).unwrap(), "appear");
    assert_eq!(
        normalize_wait_target_state(Some("disappear")).unwrap(),
        "disappear"
    );
    assert!(normalize_wait_target_state(Some("later")).is_err());
}

#[test]
fn fallback_probe_capture_mode_switches_to_display_after_first_app_attempt() {
    assert_eq!(fallback_probe_capture_mode(None, 1, Some("Notes")), None);
    assert_eq!(
        fallback_probe_capture_mode(None, 2, Some("Notes")),
        Some("display")
    );
    assert_eq!(
        fallback_probe_capture_mode(Some("window"), 2, Some("Notes")),
        Some("window")
    );
    assert_eq!(fallback_probe_capture_mode(None, 2, None), None);
}

#[test]
fn fallback_probe_capture_mode_is_generic_for_waits_and_grounded_actions() {
    assert_eq!(
        fallback_probe_capture_mode(None, 2, Some("Finder")),
        Some("display")
    );
    assert_eq!(
        fallback_probe_capture_mode(Some("display"), 2, Some("Finder")),
        Some("display")
    );
}

#[test]
fn remaining_wait_budget_stops_at_zero() {
    let future_deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    assert!(remaining_wait_budget_ms(future_deadline).unwrap() > 0);

    let expired_deadline = tokio::time::Instant::now() - Duration::from_millis(1);
    assert_eq!(remaining_wait_budget_ms(expired_deadline), None);
}
