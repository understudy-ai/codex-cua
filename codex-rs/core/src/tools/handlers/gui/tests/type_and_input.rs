use super::*;
use serial_test::serial;
use std::env;
use std::ffi::OsStr;

struct EnvVarGuard {
    key: &'static str,
    original: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &OsStr) -> Self {
        let original = env::var_os(key);
        unsafe {
            env::set_var(key, value);
        }
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.original.take() {
            Some(value) => unsafe {
                env::set_var(self.key, value);
            },
            None => unsafe {
                env::remove_var(self.key);
            },
        }
    }
}

#[test]
fn targeted_type_focus_point_prefers_display_box_center() {
    let resolved = ResolvedTarget {
        window_title: Some("Browser".to_string()),
        provider: "openai:gpt-5.4".to_string(),
        confidence: 0.94,
        reason: Some("matched the only editable field".to_string()),
        grounding_mode_requested: "complex".to_string(),
        grounding_mode_effective: "complex".to_string(),
        scope: Some("Workspace panel".to_string()),
        point: HelperPoint { x: 114.0, y: 225.0 },
        bounds: HelperRect {
            x: 80.0,
            y: 200.0,
            width: 240.0,
            height: 40.0,
        },
        local_point: None,
        local_bounds: None,
        raw: None,
        capture_state: ObserveState {
            capture: CaptureArtifact {
                origin_x: 0.0,
                origin_y: 0.0,
                width: 1280,
                height: 800,
                image_width: 1280,
                image_height: 800,

                display_index: 1,
                capture_mode: CaptureMode::Window,
                window_title: Some("Browser".to_string()),
                window_count: Some(1),
                window_capture_strategy: Some("bounds".to_string()),
                host_exclusion: HostCaptureExclusionState::default(),
            },
            app_name: Some("Browser".to_string()),
        },
    };

    let focus_point = targeted_type_focus_point(&resolved);
    assert_eq!(focus_point.x, 200.0);
    assert_eq!(focus_point.y, 220.0);
}

#[test]
fn targeted_type_focus_point_falls_back_to_grounded_point_without_valid_box() {
    let resolved = ResolvedTarget {
        window_title: Some("Browser".to_string()),
        provider: "openai:gpt-5.4".to_string(),
        confidence: 0.94,
        reason: Some("matched the only editable field".to_string()),
        grounding_mode_requested: "complex".to_string(),
        grounding_mode_effective: "complex".to_string(),
        scope: Some("Workspace panel".to_string()),
        point: HelperPoint { x: 114.0, y: 225.0 },
        bounds: HelperRect {
            x: 80.0,
            y: 200.0,
            width: 0.0,
            height: 40.0,
        },
        local_point: None,
        local_bounds: None,
        raw: None,
        capture_state: ObserveState {
            capture: CaptureArtifact {
                origin_x: 0.0,
                origin_y: 0.0,
                width: 1280,
                height: 800,
                image_width: 1280,
                image_height: 800,

                display_index: 1,
                capture_mode: CaptureMode::Window,
                window_title: Some("Browser".to_string()),
                window_count: Some(1),
                window_capture_strategy: Some("bounds".to_string()),
                host_exclusion: HostCaptureExclusionState::default(),
            },
            app_name: Some("Browser".to_string()),
        },
    };

    let focus_point = targeted_type_focus_point(&resolved);
    assert_eq!(focus_point.x, 114.0);
    assert_eq!(focus_point.y, 225.0);
}

#[test]
#[serial]
fn resolve_type_value_accepts_plain_secret_env_var_names() {
    let _guard = EnvVarGuard::set("GUI_TYPE_TEST_SECRET", OsStr::new("hunter2"));
    let args = TypeArgs {
        value: None,
        secret_env_var: Some("GUI_TYPE_TEST_SECRET".to_string()),
        secret_command_env_var: None,
        target: None,
        location_hint: None,
        scope: None,
        grounding_mode: None,
        type_strategy: None,
        capture_mode: None,
        window_title: None,
        window_selector: None,
        app: None,
        replace: None,
        submit: None,
    };

    let value = resolve_type_value(&args).expect("plain env var names should still be supported");
    assert_eq!(value, "hunter2");
}

#[test]
#[serial]
fn resolve_type_value_accepts_plain_secret_command_env_var_names() {
    let _guard = EnvVarGuard::set(
        "GUI_TYPE_TEST_SECRET_COMMAND",
        OsStr::new("printf 'from-command'"),
    );
    let args = TypeArgs {
        value: None,
        secret_env_var: None,
        secret_command_env_var: Some("GUI_TYPE_TEST_SECRET_COMMAND".to_string()),
        target: None,
        location_hint: None,
        scope: None,
        grounding_mode: None,
        type_strategy: None,
        capture_mode: None,
        window_title: None,
        window_selector: None,
        app: None,
        replace: None,
        submit: None,
    };

    let value =
        resolve_type_value(&args).expect("plain command env var names should still be supported");
    assert_eq!(value, "from-command");
}
