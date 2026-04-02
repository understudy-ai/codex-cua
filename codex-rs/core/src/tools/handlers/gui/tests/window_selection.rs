use super::*;

#[test]
fn normalize_window_selection_merges_top_level_and_selector_fields() {
    let selector = WindowSelector {
        title: None,
        title_contains: Some(" Settings ".to_string()),
        index: Some(2),
    };

    let selection = normalize_window_selection(Some(" Preferences "), Some(&selector))
        .expect("window selection should normalize")
        .expect("window selection should exist");

    assert_eq!(selection.title.as_deref(), Some("Preferences"));
    assert_eq!(selection.title_contains.as_deref(), Some("Settings"));
    assert_eq!(selection.index, Some(2));
}

#[test]
fn resolve_capture_target_uses_window_bounds_when_requested() {
    let context = HelperCaptureContext {
        app_name: Some("Notes".to_string()),
        cursor: HelperPoint { x: 10.0, y: 20.0 },
        display: HelperDisplayDescriptor {
            index: 1,
            bounds: HelperRect {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
        },
        window_id: Some(42),
        window_title: Some("Quick Note".to_string()),
        window_bounds: Some(HelperRect {
            x: 100.0,
            y: 80.0,
            width: 800.0,
            height: 600.0,
        }),
        window_count: Some(3),
        window_capture_strategy: Some("bounds".to_string()),
        host_self_exclude_applied: Some(false),
        host_frontmost_excluded: Some(false),
        host_frontmost_app_name: None,
        host_frontmost_bundle_id: None,
    };

    let capture =
        resolve_capture_target(&context, Some("window"), true, true).expect("window capture");

    assert_eq!(capture.mode, CaptureMode::Window);
    assert_eq!(capture.width, 800);
    assert_eq!(capture.height, 600);
    assert_eq!(capture.window_title.as_deref(), Some("Quick Note"));
    assert_eq!(capture.window_count, Some(3));
    assert_eq!(capture.window_capture_strategy.as_deref(), Some("bounds"));
}

#[test]
#[cfg(target_os = "macos")]
fn normalize_capture_mode_env_accepts_common_variants() {
    assert_eq!(normalize_capture_mode_env("window"), Some("window"));
    assert_eq!(normalize_capture_mode_env(" Window "), Some("window"));
    assert_eq!(normalize_capture_mode_env("display."), Some("display"));
    assert_eq!(normalize_capture_mode_env("DISPLAY"), Some("display"));
    assert_eq!(normalize_capture_mode_env("workspace"), None);
}

#[test]
fn resolve_capture_target_prefers_window_for_in_app_work_when_available() {
    let context = HelperCaptureContext {
        app_name: Some("Notes".to_string()),
        cursor: HelperPoint { x: 10.0, y: 20.0 },
        display: HelperDisplayDescriptor {
            index: 1,
            bounds: HelperRect {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
        },
        window_id: Some(42),
        window_title: Some("Quick Note".to_string()),
        window_bounds: Some(HelperRect {
            x: 100.0,
            y: 80.0,
            width: 800.0,
            height: 600.0,
        }),
        window_count: Some(1),
        window_capture_strategy: Some("bounds".to_string()),
        host_self_exclude_applied: Some(false),
        host_frontmost_excluded: Some(false),
        host_frontmost_app_name: None,
        host_frontmost_bundle_id: None,
    };

    let capture = resolve_capture_target(&context, None, false, true)
        .expect("window should be preferred for in-app work");

    assert_eq!(capture.mode, CaptureMode::Window);
    assert_eq!(capture.width, 800);
    assert_eq!(capture.height, 600);
}

#[test]
fn resolve_capture_target_adjusts_implicit_display_for_host_self_exclude() {
    let context = HelperCaptureContext {
        app_name: Some("Notes".to_string()),
        cursor: HelperPoint { x: 10.0, y: 20.0 },
        display: HelperDisplayDescriptor {
            index: 1,
            bounds: HelperRect {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
        },
        window_id: Some(42),
        window_title: Some("Quick Note".to_string()),
        window_bounds: Some(HelperRect {
            x: 100.0,
            y: 80.0,
            width: 800.0,
            height: 600.0,
        }),
        window_count: Some(1),
        window_capture_strategy: Some("bounds".to_string()),
        host_self_exclude_applied: Some(true),
        host_frontmost_excluded: Some(true),
        host_frontmost_app_name: Some("Codex".to_string()),
        host_frontmost_bundle_id: Some("com.openai.codex".to_string()),
    };

    let capture = resolve_capture_target(&context, None, false, false)
        .expect("implicit display capture should adjust to a safe window capture");

    assert_eq!(capture.mode, CaptureMode::Window);
    assert!(capture.host_self_exclude_adjusted);
    assert_eq!(capture.width, 800);
    assert_eq!(capture.height, 600);
}
