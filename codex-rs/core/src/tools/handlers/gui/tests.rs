use super::grounding::GroundingModelImageConfig;
use super::grounding::annotate_grounding_raw;
use super::grounding::annotate_grounding_round_artifacts;
use super::grounding::build_gui_grounding_prompt;
use super::grounding::build_gui_grounding_refinement_prompt;
use super::grounding::build_gui_grounding_validation_prompt;
use super::grounding::build_not_found_retry_notes;
use super::grounding::create_refinement_crop;
use super::grounding::extract_grounding_json;
use super::grounding::grounding_bbox_to_rect;
use super::grounding::image_point_to_display;
use super::grounding::image_rect_to_display;
use super::grounding::prepare_grounding_model_image;
use super::grounding::render_guide_overlay;
use super::grounding::render_validation_overlay;
use super::grounding::should_generate_retry_guide;
use super::grounding::should_use_high_resolution_refinement;
use super::grounding::translate_model_point_to_original;
use super::grounding::translate_original_point_to_model;
use super::grounding::translate_original_point_to_refinement;
use super::grounding::translate_refinement_point_to_original;
#[cfg(target_os = "macos")]
use super::platform::platform_macos::normalize_capture_mode_env;
use super::readiness::GuiEnvironmentReadinessCheck;
use super::readiness::GuiEnvironmentReadinessSnapshot;
use super::readiness::GuiReadinessStatus;
use super::readiness::GuiToolCapability;
use super::readiness::resolve_gui_runtime_capabilities;
use super::*;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::turn_diff_tracker::TurnDiffTracker;
use image::ImageFormat;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

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

    assert_eq!(capture.mode, "window");
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
fn prepare_targeted_gui_action_is_noop_without_targeting() {
    prepare_targeted_gui_action(None, None, None).expect("no-op targeted action");
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

    assert_eq!(capture.mode, "window");
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

    assert_eq!(capture.mode, "window");
    assert!(capture.host_self_exclude_adjusted);
    assert_eq!(capture.width, 800);
    assert_eq!(capture.height, 600);
}

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
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
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
        if previous.capture.capture_mode == "window" {
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
fn normalize_drag_endpoint_accepts_semantic_targets() {
    let endpoint = normalize_drag_endpoint(
        "source",
        "from_target",
        Some("Save button"),
        Some("top right"),
        Some("toolbar"),
    )
    .expect("target endpoint should normalize");

    match endpoint {
        DragEndpoint::Target {
            target,
            location_hint,
            scope,
        } => {
            assert_eq!(target, "Save button");
            assert_eq!(location_hint, Some("top right"));
            assert_eq!(scope, Some("toolbar"));
        }
    }
}

#[test]
fn normalize_drag_endpoint_requires_semantic_targets() {
    let error = normalize_drag_endpoint("destination", "to_target", None, None, None)
        .expect_err("missing target endpoint should fail");

    assert!(
        error
            .to_string()
            .contains("requires `to_target` for the destination")
    );
}

#[test]
fn normalize_grounding_mode_defaults_and_validates() {
    assert_eq!(
        normalize_grounding_mode(None, "gui_click").unwrap(),
        "single"
    );
    assert_eq!(normalize_grounding_mode(None, "type").unwrap(), "complex");
    assert_eq!(
        normalize_grounding_mode(None, "drag_source").unwrap(),
        "complex"
    );
    assert_eq!(
        normalize_grounding_mode(Some("complex"), "gui_click").unwrap(),
        "complex"
    );
    assert!(normalize_grounding_mode(Some("dense"), "gui_click").is_err());
}

#[test]
fn grounding_prompt_includes_retry_context_and_guide_hint() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800,
            height: 600,
            image_width: 1600,
            image_height: 1200,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let prompt = build_gui_grounding_prompt(
        GuiTargetRequest {
            app: Some("Notes"),
            capture_mode: Some("window"),
            window_selection: None,
            target: "Save button",
            location_hint: Some("top right"),
            scope: Some("toolbar"),
            grounding_mode: Some("complex"),
            action: "click",
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        },
        &state,
        "complex",
        &[
            "Round 1 validator rejected the candidate".to_string(),
            "Move away from the highlighted point".to_string(),
        ],
        true,
    );

    assert!(prompt.contains("Retry context:"));
    assert!(prompt.contains("Round 1 validator rejected the candidate"));
    assert!(prompt.contains("additional guide image"));
    assert!(prompt.contains("Match the target by visible meaning"));
    assert!(prompt.contains("Match subtle or weakly labeled controls"));
    assert!(prompt.contains("button, icon button, toolbar item"));
    assert!(prompt.contains("individual icon-bearing control"));
    assert!(prompt.contains("visible button label inside a dialog"));
}

#[test]
fn grounding_prompt_adds_action_specific_editable_surface_guidance() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800,
            height: 600,
            image_width: 1600,
            image_height: 1200,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let prompt = build_gui_grounding_prompt(
        GuiTargetRequest {
            app: Some("Notes"),
            capture_mode: Some("window"),
            window_selection: None,
            target: "Search field",
            location_hint: Some("top right"),
            scope: Some("toolbar"),
            grounding_mode: Some("complex"),
            action: "type",
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        },
        &state,
        "complex",
        &[],
        false,
    );

    assert!(prompt.contains("editable surface itself"));
    assert!(prompt.contains("icon-only affordance"));
}

#[test]
fn not_found_retry_notes_encourage_semantic_role_flexibility() {
    let notes = build_not_found_retry_notes(
        GuiTargetRequest {
            app: Some("Notes"),
            capture_mode: Some("window"),
            window_selection: None,
            target: "Search field",
            location_hint: Some("top right"),
            scope: Some("toolbar"),
            grounding_mode: Some("complex"),
            action: "click",
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        },
        1,
    );

    assert!(
        notes
            .iter()
            .any(|note| note.contains("Broaden the search while keeping the same semantic goal"))
    );
    assert!(notes.iter().any(|note| note.contains("visible meaning")));
    assert!(notes.iter().any(|note| note.contains("toolbar items")));
}

#[test]
fn validation_prompt_requests_failure_kind_and_retry_hint() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800,
            height: 600,
            image_width: 1600,
            image_height: 1200,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let prompt = build_gui_grounding_validation_prompt(
        GuiTargetRequest {
            app: Some("Notes"),
            capture_mode: Some("window"),
            window_selection: None,
            target: "Save button",
            location_hint: Some("top right"),
            scope: Some("toolbar"),
            grounding_mode: Some("complex"),
            action: "click",
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        },
        &state,
        &HelperPoint { x: 320.0, y: 180.0 },
        Some(&GroundingBoundingBox {
            x1: 280.0,
            y1: 140.0,
            x2: 360.0,
            y2: 220.0,
        }),
        false,
    );

    assert!(prompt.contains("failure_kind"));
    assert!(prompt.contains("retry_hint"));
    assert!(prompt.contains("wrong_region"));
    assert!(prompt.contains("whitespace, padding, decoration"));
    assert!(prompt.contains("subtle, tightly packed, or low-contrast controls"));
    assert!(prompt.contains("strongest visible semantic match"));
}

#[test]
fn validation_prompt_mentions_zoomed_crop_when_requested() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 100.0,
            origin_y: 200.0,
            width: 360,
            height: 320,
            image_width: 1200,
            image_height: 1067,
            scale_x: 3.333,
            scale_y: 3.334,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let prompt = build_gui_grounding_validation_prompt(
        GuiTargetRequest {
            app: Some("Notes"),
            capture_mode: Some("window"),
            window_selection: None,
            target: "Pin icon button",
            location_hint: Some("toolbar row"),
            scope: Some("toolbar"),
            grounding_mode: Some("complex"),
            action: "click",
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        },
        &state,
        &HelperPoint { x: 640.0, y: 400.0 },
        None,
        true,
    );

    assert!(prompt.contains("zoomed crop"));
    assert!(prompt.contains("original request"));
}

#[test]
fn retry_guide_is_suppressed_for_wrong_region_and_scope_mismatch() {
    assert!(!should_generate_retry_guide(Some("wrong_region")));
    assert!(!should_generate_retry_guide(Some("scope_mismatch")));
    assert!(should_generate_retry_guide(Some("wrong_control")));
    assert!(should_generate_retry_guide(None));
}

#[test]
fn refinement_prompt_mentions_zoomed_crop_context() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800,
            height: 600,
            image_width: 1600,
            image_height: 1200,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };
    let crop = create_refinement_crop(
        &{
            let base = image::RgbaImage::from_pixel(1200, 900, image::Rgba([240, 240, 240, 255]));
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(base)
                .write_to(&mut encoded, ImageFormat::Png)
                .expect("base image should encode");
            encoded.into_inner()
        },
        &HelperPoint { x: 420.0, y: 360.0 },
        Some(&GroundingBoundingBox {
            x1: 400.0,
            y1: 340.0,
            x2: 440.0,
            y2: 380.0,
        }),
    )
    .expect("refinement crop should build")
    .expect("refinement crop should exist");

    let prompt = build_gui_grounding_refinement_prompt(
        GuiTargetRequest {
            app: Some("Notes"),
            capture_mode: Some("window"),
            window_selection: None,
            target: "Save button",
            location_hint: Some("top right"),
            scope: Some("toolbar"),
            grounding_mode: Some("complex"),
            action: "click",
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        },
        &state,
        &crop,
        Some(&HelperPoint { x: 100.0, y: 120.0 }),
        Some(&GroundingBoundingBox {
            x1: 80.0,
            y1: 100.0,
            x2: 140.0,
            y2: 160.0,
        }),
    );

    assert!(prompt.contains("zoomed crop around a previous candidate"));
    assert!(prompt.contains("Previous crop-relative point"));
    assert!(prompt.contains("Refine the target inside this crop"));
}

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
fn local_point_within_state_reports_only_in_bounds_targets() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 100.0,
            origin_y: 200.0,
            width: 400,
            height: 300,
            image_width: 800,
            image_height: 600,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let in_bounds = local_point_within_state(&state, &HelperPoint { x: 125.0, y: 250.0 })
        .expect("point should be within capture");
    assert_eq!(in_bounds.x, 25.0);
    assert_eq!(in_bounds.y, 50.0);
    assert!(local_point_within_state(&state, &HelperPoint { x: 50.0, y: 250.0 }).is_none());
}

#[test]
fn image_space_helpers_validate_capture_relative_geometry() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 100.0,
            origin_y: 200.0,
            width: 400,
            height: 300,
            image_width: 800,
            image_height: 600,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let point = image_point_within_capture(&state, &HelperPoint { x: 125.0, y: 250.0 })
        .expect("point should remain in image space");
    assert_eq!(point.x, 125.0);
    assert_eq!(point.y, 250.0);
    assert!(image_point_within_capture(&state, &HelperPoint { x: 850.0, y: 250.0 }).is_none());

    let rect = local_rect_within_state(
        &state,
        &HelperRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 80.0,
        },
    )
    .expect("rect should fit inside capture");
    assert_eq!(rect.width, 100.0);
    assert_eq!(rect.height, 80.0);
    assert!(
        local_rect_within_state(
            &state,
            &HelperRect {
                x: 750.0,
                y: 20.0,
                width: 100.0,
                height: 80.0,
            },
        )
        .is_none()
    );
}

#[test]
fn image_space_geometry_maps_back_to_display_space() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 100.0,
            origin_y: 200.0,
            width: 400,
            height: 300,
            image_width: 800,
            image_height: 600,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };

    let display_point = image_point_to_display(&state, &HelperPoint { x: 300.0, y: 120.0 });
    assert_eq!(display_point.x, 250.0);
    assert_eq!(display_point.y, 260.0);

    let display_rect = image_rect_to_display(
        &state,
        &HelperRect {
            x: 200.0,
            y: 100.0,
            width: 160.0,
            height: 80.0,
        },
    );
    assert_eq!(display_rect.x, 200.0);
    assert_eq!(display_rect.y, 250.0);
    assert_eq!(display_rect.width, 80.0);
    assert_eq!(display_rect.height, 40.0);
}

#[test]
fn grounding_model_image_normalizes_hidpi_capture_dimensions() {
    let base = image::RgbaImage::from_pixel(1600, 1200, image::Rgba([240, 240, 240, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(base)
        .write_to(&mut encoded, ImageFormat::Png)
        .expect("base image should encode");

    let prepared = prepare_grounding_model_image(
        &encoded.into_inner(),
        GroundingModelImageConfig {
            logical_width: Some(800),
            logical_height: Some(600),
            scale_x: Some(2.0),
            scale_y: Some(2.0),
            allow_logical_normalization: true,
        },
        "test image",
    )
    .expect("prepared image should build");

    assert_eq!(prepared.original_width, 1600);
    assert_eq!(prepared.original_height, 1200);
    assert_eq!(prepared.working_width, 800);
    assert_eq!(prepared.working_height, 600);
    assert_eq!(prepared.model_width, 800);
    assert_eq!(prepared.model_height, 600);
    assert_eq!(prepared.mime_type, "image/png");
    assert!(prepared.logical_normalization_applied);
    assert!((prepared.model_to_original_scale_x - 2.0).abs() < f64::EPSILON);
    assert!((prepared.model_to_original_scale_y - 2.0).abs() < f64::EPSILON);
}

#[test]
fn grounding_model_image_roundtrips_between_model_and_original_space() {
    let base = image::RgbaImage::from_pixel(1600, 1200, image::Rgba([240, 240, 240, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(base)
        .write_to(&mut encoded, ImageFormat::Png)
        .expect("base image should encode");
    let prepared = prepare_grounding_model_image(
        &encoded.into_inner(),
        GroundingModelImageConfig {
            logical_width: Some(800),
            logical_height: Some(600),
            scale_x: Some(2.0),
            scale_y: Some(2.0),
            allow_logical_normalization: true,
        },
        "test image",
    )
    .expect("prepared image should build");
    let original_point = HelperPoint { x: 640.0, y: 420.0 };

    let model_point = translate_original_point_to_model(&prepared, &original_point);
    let roundtrip_point = translate_model_point_to_original(&prepared, &model_point);

    assert!((model_point.x - 320.0).abs() < 0.01);
    assert!((model_point.y - 210.0).abs() < 0.01);
    assert!((roundtrip_point.x - original_point.x).abs() < 0.01);
    assert!((roundtrip_point.y - original_point.y).abs() < 0.01);
}

#[test]
fn refinement_is_used_for_tiny_targets_and_maps_back_to_original_space() {
    let state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: 800,
            height: 600,
            image_width: 1600,
            image_height: 1200,
            scale_x: 2.0,
            scale_y: 2.0,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("selected_window".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Notes".to_string()),
    };
    let original_point = HelperPoint { x: 640.0, y: 420.0 };
    let tiny_bbox = GroundingBoundingBox {
        x1: 628.0,
        y1: 408.0,
        x2: 652.0,
        y2: 432.0,
    };

    assert!(should_use_high_resolution_refinement(
        &state,
        &original_point,
        Some(&tiny_bbox)
    ));

    let base = image::RgbaImage::from_pixel(1600, 1200, image::Rgba([240, 240, 240, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(base)
        .write_to(&mut encoded, ImageFormat::Png)
        .expect("base image should encode");
    let crop = create_refinement_crop(&encoded.into_inner(), &original_point, Some(&tiny_bbox))
        .expect("refinement crop should build")
        .expect("refinement crop should exist");

    let refinement_point = translate_original_point_to_refinement(&crop, &original_point);
    let roundtrip_point = translate_refinement_point_to_original(&crop, &refinement_point);

    assert!((roundtrip_point.x - original_point.x).abs() < 0.51);
    assert!((roundtrip_point.y - original_point.y).abs() < 0.51);
    assert!(crop.model_width >= crop.crop_width);
    assert!(crop.model_height >= crop.crop_height);
}

#[test]
fn grounding_helpers_extract_json_and_convert_bounding_boxes() {
    let wrapped = "```json\n{\"status\":\"resolved\",\"found\":true}\n```";
    assert_eq!(
        extract_grounding_json(wrapped),
        Some("{\"status\":\"resolved\",\"found\":true}")
    );

    let rect = grounding_bbox_to_rect(&GroundingBoundingBox {
        x1: 10.0,
        y1: 20.0,
        x2: 30.0,
        y2: 60.0,
    })
    .expect("bbox should convert");
    assert_eq!(rect.x, 10.0);
    assert_eq!(rect.y, 20.0);
    assert_eq!(rect.width, 20.0);
    assert_eq!(rect.height, 40.0);
    assert!(
        grounding_bbox_to_rect(&GroundingBoundingBox {
            x1: 30.0,
            y1: 20.0,
            x2: 10.0,
            y2: 60.0,
        })
        .is_none()
    );
}

#[test]
fn grounding_validation_overlay_preserves_image_dimensions() {
    let base = image::RgbaImage::from_pixel(32, 24, image::Rgba([240, 240, 240, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(base.clone())
        .write_to(&mut encoded, ImageFormat::Png)
        .expect("base image should encode");
    let base_png = encoded.into_inner();

    let overlay = render_validation_overlay(
        &base_png,
        &HelperPoint { x: 12.0, y: 8.0 },
        Some(&GroundingBoundingBox {
            x1: 4.0,
            y1: 5.0,
            x2: 20.0,
            y2: 16.0,
        }),
    )
    .expect("overlay should render");
    let rendered = image::load_from_memory(&overlay).expect("overlay should decode");

    assert_eq!(rendered.width(), 32);
    assert_eq!(rendered.height(), 24);
    assert_ne!(overlay, base_png);
}

#[test]
fn grounding_guide_overlay_preserves_image_dimensions() {
    let base = image::RgbaImage::from_pixel(32, 24, image::Rgba([240, 240, 240, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(base.clone())
        .write_to(&mut encoded, ImageFormat::Png)
        .expect("base image should encode");
    let base_png = encoded.into_inner();

    let overlay = render_guide_overlay(
        &base_png,
        &HelperPoint { x: 14.0, y: 10.0 },
        Some(&GroundingBoundingBox {
            x1: 6.0,
            y1: 4.0,
            x2: 18.0,
            y2: 20.0,
        }),
    )
    .expect("guide overlay should render");
    let rendered = image::load_from_memory(&overlay).expect("guide overlay should decode");

    assert_eq!(rendered.width(), 32);
    assert_eq!(rendered.height(), 24);
    assert_ne!(overlay, base_png);
}

#[test]
fn annotate_grounding_raw_adds_validation_metadata() {
    let mut raw = serde_json::json!({
        "status": "resolved",
        "found": true,
    });

    annotate_grounding_raw(
        &mut raw,
        "complex",
        "validated",
        true,
        "approved",
        Some("marker aligned with the target"),
        Some(0.92),
        2,
    );

    assert_eq!(raw["grounding_mode_requested"], "complex");
    assert_eq!(raw["grounding_mode_effective"], "complex");
    assert_eq!(raw["selected_attempt"], "validated");
    assert_eq!(raw["grounding_validation_triggered"], true);
    assert_eq!(raw["grounding_rounds_attempted"], 2);
    assert_eq!(raw["validation"]["status"], "approved");
    assert_eq!(
        raw["validation"]["reason"],
        "marker aligned with the target"
    );
}

#[test]
fn annotate_grounding_round_artifacts_adds_structured_attempts() {
    let mut raw = serde_json::json!({
        "status": "resolved",
        "found": true,
    });

    annotate_grounding_round_artifacts(
        &mut raw,
        &[
            serde_json::json!({
                "round": 1,
                "terminal_state": "validator_rejected_retry",
                "predictor": {
                    "outcome": {
                        "status": "resolved",
                    },
                },
            }),
            serde_json::json!({
                "round": 2,
                "terminal_state": "accepted",
                "validation": {
                    "triggered": true,
                    "outcome": {
                        "status": "approved",
                    },
                },
            }),
        ],
    );

    assert_eq!(raw["grounding_round_artifacts"][0]["round"], 1);
    assert_eq!(
        raw["grounding_round_artifacts"][0]["terminal_state"],
        "validator_rejected_retry"
    );
    assert_eq!(raw["grounding_round_artifacts"][1]["round"], 2);
    assert_eq!(
        raw["grounding_round_artifacts"][1]["validation"]["triggered"],
        true
    );
}

#[test]
fn target_resolution_details_surface_grounding_diagnostics_summary() {
    let grounded = GroundedGuiTarget {
        grounding_method: "grounding",
        resolved: ResolvedTarget {
            window_title: Some("Finder".to_string()),
            provider: "openai:gpt".to_string(),
            confidence: 0.92,
            reason: Some("matched visible search affordance".to_string()),
            grounding_mode_requested: "complex".to_string(),
            grounding_mode_effective: "complex".to_string(),
            scope: Some("toolbar".to_string()),
            point: HelperPoint { x: 1200.0, y: 42.0 },
            bounds: HelperRect {
                x: 1180.0,
                y: 22.0,
                width: 48.0,
                height: 32.0,
            },
            local_point: Some(HelperPoint { x: 600.0, y: 21.0 }),
            local_bounds: Some(HelperRect {
                x: 590.0,
                y: 11.0,
                width: 24.0,
                height: 16.0,
            }),
            raw: Some(serde_json::json!({
                "selected_attempt": "validated_retry",
                "grounding_rounds_attempted": 2,
                "grounding_validation_triggered": true,
                "grounding_model_image": {
                    "model_width": 800,
                    "model_height": 600,
                },
                "validation": {
                    "status": "approved",
                },
                "grounding_round_artifacts": [
                    {
                        "round": 1,
                        "terminal_state": "validator_rejected_retry",
                    },
                    {
                        "round": 2,
                        "terminal_state": "accepted",
                    },
                ],
            })),
            capture_state: ObserveState {
                capture: CaptureArtifact {
                    origin_x: 0.0,
                    origin_y: 0.0,
                    width: 1440,
                    height: 900,
                    image_width: 1440,
                    image_height: 900,
                    scale_x: 1.0,
                    scale_y: 1.0,
                    display_index: 1,
                    capture_mode: "display",
                    window_title: Some("Finder".to_string()),
                    window_count: Some(1),
                    window_capture_strategy: Some("display".to_string()),
                    host_exclusion: HostCaptureExclusionState::default(),
                },
                app_name: Some("Finder".to_string()),
            },
        },
    };

    let details = build_target_resolution_details("Search", &grounded);

    assert_eq!(
        details["grounding_diagnostics"]["selected_attempt"],
        "validated_retry"
    );
    assert_eq!(details["grounding_diagnostics"]["rounds_attempted"], 2);
    assert_eq!(
        details["grounding_diagnostics"]["round_artifacts"][0]["terminal_state"],
        "validator_rejected_retry"
    );
    assert_eq!(
        details["grounding_diagnostics"]["model_image"]["model_width"],
        800
    );
}

#[test]
fn runtime_capabilities_disable_grounded_actions_without_grounding() {
    let readiness = GuiEnvironmentReadinessSnapshot {
        status: "ready",
        checks: vec![
            GuiEnvironmentReadinessCheck {
                id: "platform",
                label: "Platform",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "accessibility",
                label: "Accessibility",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "screen_recording",
                label: "Screen Recording",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "native_helper",
                label: "Native GUI Helper",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
        ],
    };
    let capabilities = resolve_gui_runtime_capabilities(false, &readiness, None);
    assert!(capabilities.platform_supported);
    assert!(!capabilities.grounding_available);
    assert!(capabilities.native_helper_available);
    assert!(capabilities.screen_capture_available);
    assert!(capabilities.input_available);
    assert!(capabilities.enabled_tool_names.contains(&"gui_scroll"));
    assert!(capabilities.disabled_tool_names.contains(&"gui_click"));
    assert!(!capabilities.tool_availability["gui_click"].enabled);
    assert!(capabilities.tool_availability["gui_scroll"].enabled);
    assert!(capabilities.tool_availability["gui_scroll"].targetless_only);
    assert!(capabilities.tool_availability["gui_type"].enabled);
    assert!(capabilities.tool_availability["gui_type"].targetless_only);
}

#[test]
fn runtime_capabilities_disable_input_actions_without_accessibility() {
    let readiness = GuiEnvironmentReadinessSnapshot {
        status: "blocked",
        checks: vec![
            GuiEnvironmentReadinessCheck {
                id: "platform",
                label: "Platform",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "accessibility",
                label: "Accessibility",
                status: GuiReadinessStatus::Error,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "screen_recording",
                label: "Screen Recording",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "native_helper",
                label: "Native GUI Helper",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
        ],
    };
    let capabilities = resolve_gui_runtime_capabilities(true, &readiness, None);
    assert!(capabilities.platform_supported);
    assert!(capabilities.grounding_available);
    assert!(capabilities.native_helper_available);
    assert!(capabilities.screen_capture_available);
    assert!(!capabilities.input_available);
    assert!(!capabilities.tool_availability["gui_click"].enabled);
    assert!(!capabilities.tool_availability["gui_key"].enabled);
    assert!(capabilities.tool_availability["gui_observe"].enabled);
    assert!(capabilities.tool_availability["gui_wait"].enabled);
}

#[test]
fn runtime_capabilities_respect_platform_tool_contract() {
    let readiness = GuiEnvironmentReadinessSnapshot {
        status: "ready",
        checks: vec![
            GuiEnvironmentReadinessCheck {
                id: "platform",
                label: "Platform",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "accessibility",
                label: "Accessibility",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "screen_recording",
                label: "Screen Recording",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
            GuiEnvironmentReadinessCheck {
                id: "native_helper",
                label: "Native GUI Helper",
                status: GuiReadinessStatus::Ok,
                summary: String::new(),
                detail: None,
            },
        ],
    };
    let platform_support = HashMap::from([
        (
            "gui_move",
            GuiToolCapability {
                enabled: false,
                reason: Some("platform backend does not support pointer movement".to_string()),
                targetless_only: false,
            },
        ),
        (
            "gui_type",
            GuiToolCapability {
                enabled: true,
                reason: Some(
                    "platform backend only supports typing into the focused control".to_string(),
                ),
                targetless_only: true,
            },
        ),
    ]);

    let capabilities = resolve_gui_runtime_capabilities(true, &readiness, Some(&platform_support));

    assert!(!capabilities.tool_availability["gui_move"].enabled);
    assert_eq!(
        capabilities.tool_availability["gui_move"].reason.as_deref(),
        Some("platform backend does not support pointer movement")
    );
    assert!(capabilities.tool_availability["gui_type"].enabled);
    assert!(capabilities.tool_availability["gui_type"].targetless_only);
    assert_eq!(
        capabilities.tool_availability["gui_type"].reason.as_deref(),
        Some("platform backend only supports typing into the focused control")
    );
}

fn gui_invocation(
    session: Arc<crate::codex::Session>,
    turn: Arc<crate::codex::TurnContext>,
    tool_name: &str,
    args: serde_json::Value,
) -> ToolInvocation {
    ToolInvocation {
        session,
        turn,
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "gui-test-call".to_string(),
        tool_name: tool_name.to_string(),
        tool_namespace: None,
        payload: ToolPayload::Function {
            arguments: args.to_string(),
        },
    }
}

#[cfg(target_os = "macos")]
fn run_applescript(script: &str) -> String {
    let output = std::process::Command::new("osascript")
        .args(["-l", "AppleScript", "-e", script])
        .output()
        .expect("osascript should launch");
    if !output.status.success() {
        panic!(
            "AppleScript failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(target_os = "macos")]
fn close_textedit_without_saving() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-l",
            "AppleScript",
            "-e",
            r#"
tell application "TextEdit"
	if it is running then
		repeat with docRef in documents
			close docRef saving no
		end repeat
		activate
	end if
end tell
"#,
        ])
        .output();
}

#[cfg(target_os = "macos")]
fn read_textedit_document_text() -> String {
    run_applescript(
        r#"
tell application "TextEdit"
	if (count of documents) is 0 then return ""
	return text of document 1
end tell
"#,
    )
}

#[cfg(target_os = "macos")]
fn launch_textedit() {
    let status = std::process::Command::new("open")
        .args(["-a", "TextEdit"])
        .status()
        .expect("open should launch TextEdit");
    assert!(status.success(), "open -a TextEdit should succeed");
    std::thread::sleep(std::time::Duration::from_millis(400));
    run_applescript(r#"tell application "TextEdit" to activate"#);
}

#[cfg(target_os = "macos")]
fn wait_for_textedit_document_text(expected: &str) {
    let started_at = std::time::Instant::now();
    loop {
        let text = read_textedit_document_text();
        if text.contains(expected) {
            return;
        }
        assert!(
            started_at.elapsed() < std::time::Duration::from_secs(10),
            "timed out waiting for TextEdit content `{expected}`, last content was `{text}`"
        );
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
fn macos_gui_capture_smoke_test() {
    let helper_path = resolve_helper_binary().expect("native GUI helper should compile");
    assert!(helper_path.exists(), "helper binary should exist");

    let observation =
        observe_platform(None, false, Some("display"), None, false).expect("display capture");

    assert!(
        !observation.image_bytes.is_empty(),
        "captured image should not be empty"
    );
    assert_eq!(observation.state.capture.capture_mode, "display");
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Accessibility permissions"]
fn macos_gui_move_cursor_smoke_test() {
    let context = capture_context(None, false, None).expect("capture context should be available");

    run_gui_event(
        "move_cursor",
        None,
        &[
            ("CODEX_GUI_X", context.cursor.x),
            ("CODEX_GUI_Y", context.cursor.y),
        ],
        &[("CODEX_GUI_SETTLE_MS", "1".to_string())],
    )
    .expect("move cursor event should succeed");
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Accessibility permissions"]
async fn macos_gui_move_tool_handler_smoke_test() {
    let context = capture_context(None, false, None).expect("capture context should be available");
    let (session, turn) = crate::codex::make_session_and_context().await;
    let handler = GuiHandler::default();
    let payload = serde_json::json!({
        "x": context.cursor.x,
        "y": context.cursor.y,
    });

    let output = handler
        .handle(gui_invocation(
            Arc::new(session),
            Arc::new(turn),
            "gui_move",
            payload,
        ))
        .await
        .expect("gui_move should succeed through the tool handler");

    assert!(output.success);
    assert_eq!(output.code_result["action_kind"], "move_cursor");
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
fn macos_gui_wait_smoke_test() {
    std::thread::sleep(std::time::Duration::from_millis(1));

    let context = capture_context(None, false, None).expect("capture context should be available");
    let capture =
        resolve_capture_target(&context, Some("display"), false, false).expect("display capture");
    let image_bytes =
        capture_region(&capture.bounds, capture.width, capture.height).expect("screenshot");

    assert!(
        !image_bytes.is_empty(),
        "refreshed image should not be empty"
    );
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
async fn macos_gui_textedit_typing_smoke_test() {
    close_textedit_without_saving();
    launch_textedit();

    let handler = GuiHandler::default();
    let (session, turn) = crate::codex::make_session_and_context().await;
    let session = Arc::new(session);
    let turn = Arc::new(turn);

    let outcome = async {
        let new_doc = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("gui_key should create a new TextEdit document");
        assert!(new_doc.success);
        assert_eq!(new_doc.code_result["action_kind"], "key_press");

        let type_first_line = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Codex native smoke",
                }),
            ))
            .await
            .expect("gui_type should type into the new document");
        assert!(type_first_line.success);
        assert_eq!(type_first_line.code_result["action_kind"], "type_text");
        wait_for_textedit_document_text("Codex native smoke");

        let enter = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "Enter",
                }),
            ))
            .await
            .expect("gui_key should press Enter");
        assert!(enter.success);
        assert_eq!(enter.code_result["action_kind"], "key_press");

        let type_second_line = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Second line",
                    "replace": false,
                }),
            ))
            .await
            .expect("gui_type should append the second line");
        assert!(type_second_line.success);
        wait_for_textedit_document_text("Codex native smoke\nSecond line");

        let observation = handler
            .handle(gui_invocation(
                session,
                turn,
                "gui_observe",
                serde_json::json!({
                    "app": "TextEdit",
                }),
            ))
            .await
            .expect("gui_observe should capture a refreshed TextEdit screenshot");
        assert!(observation.success);
        assert_eq!(observation.code_result["capture_mode"], "window");
        assert_eq!(observation.code_result["app"], "TextEdit");
        assert!(observation.code_result["image_url"].is_string());

        let final_text = read_textedit_document_text();
        assert!(final_text.contains("Codex native smoke\nSecond line"));
    }
    .await;

    close_textedit_without_saving();
    outcome
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroundingBenchmarkCase {
    id: String,
    element_id: Option<String>,
    target: String,
    scope: Option<String>,
    action: String,
    location_hint: Option<String>,
    difficulty: String,
    prompt_clarity: String,
    kind: String,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize)]
struct GroundingBenchmarkArtifacts {
    truths: Vec<GroundingBenchmarkTruth>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroundingBenchmarkTruth {
    id: String,
    element_id: Option<String>,
    target: String,
    scope: Option<String>,
    action: String,
    location_hint: Option<String>,
    difficulty: String,
    prompt_clarity: String,
    kind: String,
    #[serde(rename = "box")]
    box_: GroundingBenchmarkRect,
    point: HelperPoint,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize)]
struct GroundingBenchmarkRect {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug)]
struct GroundingBenchmarkMeasurement {
    case_id: String,
    kind: String,
    prompt_clarity: String,
    difficulty: String,
    found: bool,
    inside: bool,
    distance_px: f64,
    elapsed_ms: u128,
    error: Option<String>,
}

#[cfg(target_os = "macos")]
struct RenderedGroundingBenchmark {
    _tempdir: tempfile::TempDir,
    screenshot_path: std::path::PathBuf,
    screenshot_bytes: Vec<u8>,
    truths: Vec<GroundingBenchmarkTruth>,
    image_width: u32,
    image_height: u32,
    logical_width: u32,
    logical_height: u32,
    scale_x: f64,
    scale_y: f64,
}

#[cfg(target_os = "macos")]
fn understudy_grounding_fixture_source_path() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../understudy/apps/cli/src/commands/__tests__/gui-benchmark-fixture.ts");
    assert!(
        path.exists(),
        "expected Understudy grounding fixture at {}",
        path.display()
    );
    path
}

#[cfg(target_os = "macos")]
fn extract_understudy_grounding_fixture_html(source: &str) -> String {
    let marker = "export const GUI_GROUNDING_BENCHMARK_HTML = String.raw`";
    let start = source
        .find(marker)
        .map(|index| index + marker.len())
        .expect("fixture html marker should exist");
    let end_marker = "`;\n\nexport async function prepareGuiGroundingBenchmarkPage";
    let end = source[start..]
        .find(end_marker)
        .map(|index| start + index)
        .expect("fixture html closing marker should exist");
    source[start..end].to_string()
}

#[cfg(target_os = "macos")]
fn extract_understudy_grounding_benchmark_cases(source: &str) -> Vec<GroundingBenchmarkCase> {
    let marker = "export const GUI_GROUNDING_BENCHMARK_CASES: GuiGroundingBenchmarkCase[] = [";
    let start = source
        .find(marker)
        .map(|index| index + marker.len())
        .expect("benchmark cases marker should exist");
    let end = source[start..]
        .find("\n];\n\nexport const GUI_GROUNDING_BENCHMARK_HTML")
        .map(|index| start + index)
        .expect("benchmark cases closing marker should exist");
    let body = &source[start..end];

    let mut cases = Vec::new();
    let mut current: HashMap<String, String> = HashMap::new();
    let mut in_object = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "{" {
            in_object = true;
            current.clear();
            continue;
        }
        if trimmed == "}," || trimmed == "}" {
            if in_object {
                let action = current
                    .remove("action")
                    .expect("benchmark case should define action");
                let parsed = GroundingBenchmarkCase {
                    id: current
                        .remove("id")
                        .expect("benchmark case should define id"),
                    element_id: current.remove("elementId"),
                    target: current
                        .remove("target")
                        .expect("benchmark case should define target"),
                    scope: current.remove("scope"),
                    action,
                    location_hint: current.remove("locationHint"),
                    difficulty: current
                        .remove("difficulty")
                        .expect("benchmark case should define difficulty"),
                    prompt_clarity: current
                        .remove("promptClarity")
                        .expect("benchmark case should define promptClarity"),
                    kind: current
                        .remove("kind")
                        .expect("benchmark case should define kind"),
                };
                cases.push(parsed);
            }
            current.clear();
            in_object = false;
            continue;
        }
        if !in_object || trimmed.is_empty() {
            continue;
        }
        let Some((raw_key, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = raw_key.trim().to_string();
        let value = raw_value
            .trim()
            .trim_end_matches(',')
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .expect("benchmark case values should be quoted strings")
            .to_string();
        current.insert(key, value);
    }

    cases
}

#[cfg(target_os = "macos")]
fn resolve_grounding_benchmark_renderer_binary() -> std::path::PathBuf {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/tools/handlers/gui/benchmark_renderer.swift");
    let output_path = std::env::temp_dir().join("codex-gui-benchmark-renderer");
    let should_compile = std::fs::metadata(&output_path)
        .and_then(|binary| {
            let binary_mtime = binary.modified()?;
            let source_mtime = std::fs::metadata(&source_path)?.modified()?;
            Ok(source_mtime > binary_mtime)
        })
        .unwrap_or(true);
    if should_compile {
        let output = std::process::Command::new("swiftc")
            .arg(&source_path)
            .arg("-o")
            .arg(&output_path)
            .output()
            .expect("swiftc should launch for benchmark renderer");
        if !output.status.success() {
            panic!(
                "failed to compile benchmark renderer: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    output_path
}

#[cfg(target_os = "macos")]
fn render_understudy_grounding_benchmark() -> RenderedGroundingBenchmark {
    let fixture_source = std::fs::read_to_string(understudy_grounding_fixture_source_path())
        .expect("should read Understudy grounding fixture source");
    let html = extract_understudy_grounding_fixture_html(&fixture_source);
    let cases = extract_understudy_grounding_benchmark_cases(&fixture_source);

    let tempdir = tempfile::tempdir().expect("benchmark tempdir");
    let html_path = tempdir.path().join("grounding-benchmark.html");
    let cases_path = tempdir.path().join("grounding-benchmark-cases.json");
    let screenshot_path = tempdir.path().join("grounding-benchmark.png");
    let truths_path = tempdir.path().join("grounding-benchmark-truths.json");

    std::fs::write(&html_path, html).expect("write benchmark html");
    std::fs::write(
        &cases_path,
        serde_json::to_vec(&cases).expect("serialize benchmark cases"),
    )
    .expect("write benchmark cases json");

    let renderer = resolve_grounding_benchmark_renderer_binary();
    let output = std::process::Command::new(renderer)
        .arg(&html_path)
        .arg(&cases_path)
        .arg(&screenshot_path)
        .arg(&truths_path)
        .output()
        .expect("benchmark renderer should launch");
    assert!(
        output.status.success(),
        "benchmark renderer failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let screenshot_bytes = std::fs::read(&screenshot_path).expect("read benchmark screenshot");
    let screenshot =
        image::load_from_memory(&screenshot_bytes).expect("benchmark screenshot should decode");
    let artifacts: GroundingBenchmarkArtifacts =
        serde_json::from_slice(&std::fs::read(&truths_path).expect("read benchmark truths"))
            .expect("benchmark truths should deserialize");
    let logical_width = 1280_u32;
    let uniform_scale = screenshot.width() as f64 / logical_width as f64;
    let logical_height = (screenshot.height() as f64 / uniform_scale).round() as u32;

    RenderedGroundingBenchmark {
        _tempdir: tempdir,
        screenshot_path,
        screenshot_bytes,
        truths: artifacts.truths,
        image_width: screenshot.width(),
        image_height: screenshot.height(),
        logical_width,
        logical_height,
        scale_x: uniform_scale,
        scale_y: uniform_scale,
    }
}

#[cfg(target_os = "macos")]
fn developer_codex_home() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        return std::path::PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|path| path.join(".codex"))
        .expect("HOME should be set")
}

#[cfg(target_os = "macos")]
fn load_live_codex_auth() -> crate::auth::CodexAuth {
    let auth_manager = crate::AuthManager::shared(developer_codex_home(), true, Default::default());
    auth_manager
        .auth_cached()
        .expect("Codex auth is required for the live grounding benchmark")
}

#[cfg(target_os = "macos")]
async fn live_grounding_benchmark_session()
-> (Arc<crate::codex::Session>, Arc<crate::codex::TurnContext>) {
    let codex_home = tempfile::tempdir().expect("codex home tempdir");
    let codex_home_path = codex_home.keep();
    let mut config = crate::config::ConfigBuilder::default()
        .codex_home(codex_home_path)
        .build()
        .await
        .expect("build benchmark config");
    config.model = Some("gpt-5.4".to_string());
    let _ = config.features.enable(codex_features::Feature::GuiTools);

    let thread_manager = crate::ThreadManager::with_models_provider_for_tests(
        load_live_codex_auth(),
        crate::ModelProviderInfo::create_openai_provider(None),
    );
    let thread = thread_manager
        .start_thread(config)
        .await
        .expect("start live benchmark thread");
    let session = thread.thread.codex.session.clone();
    let turn = session.new_default_turn().await;
    (session, turn)
}

#[cfg(target_os = "macos")]
fn point_inside_benchmark_box(point: &HelperPoint, box_: &GroundingBenchmarkRect) -> bool {
    let left = box_.x as f64 - 4.0;
    let right = (box_.x + box_.width) as f64 + 4.0;
    let top = box_.y as f64 - 4.0;
    let bottom = (box_.y + box_.height) as f64 + 4.0;
    point.x >= left && point.x <= right && point.y >= top && point.y <= bottom
}

#[cfg(target_os = "macos")]
fn allowed_point_distance_px(truth: &GroundingBenchmarkTruth) -> f64 {
    160.0_f64.min(24.0_f64.max((truth.box_.width.max(truth.box_.height) as f64) * 0.45))
}

#[cfg(target_os = "macos")]
fn summarize_grounding_bucket(
    measurements: &[GroundingBenchmarkMeasurement],
    label: &str,
    filter: impl Fn(&GroundingBenchmarkMeasurement) -> bool,
) {
    let bucket: Vec<_> = measurements
        .iter()
        .filter(|measurement| filter(measurement))
        .collect();
    if bucket.is_empty() {
        return;
    }
    let inside = bucket
        .iter()
        .filter(|measurement| measurement.inside)
        .count();
    let found = bucket
        .iter()
        .filter(|measurement| measurement.found)
        .count();
    let avg_latency_ms = bucket
        .iter()
        .map(|measurement| measurement.elapsed_ms as f64)
        .sum::<f64>()
        / bucket.len() as f64;
    println!(
        "[codex-grounding-benchmark] bucket={} total={} found={} inside={} avg={}ms",
        label,
        bucket.len(),
        found,
        inside,
        avg_latency_ms.round()
    );
}

#[cfg(target_os = "macos")]
fn requested_grounding_case_ids() -> Option<std::collections::HashSet<String>> {
    let raw = std::env::var("CODEX_GUI_GROUNDING_CASE_IDS").ok()?;
    let ids = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<std::collections::HashSet<_>>();
    (!ids.is_empty()).then_some(ids)
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual real grounding benchmark requiring local Codex auth and swiftc"]
async fn macos_gui_grounding_benchmark_matches_understudy_thresholds() {
    const MIN_TOTAL_INSIDE_HIT_RATE: f64 = 0.78;
    const MIN_EXPLICIT_INSIDE_HIT_RATE: f64 = 0.90;
    const MIN_AMBIGUOUS_INSIDE_HIT_RATE: f64 = 0.60;
    const MIN_COMPLEX_EXPLICIT_INSIDE_HIT_RATE: f64 = 0.85;

    let rendered = render_understudy_grounding_benchmark();
    let requested_case_ids = requested_grounding_case_ids();
    let truths = rendered
        .truths
        .iter()
        .filter(|truth| match requested_case_ids.as_ref() {
            Some(ids) => ids.contains(&truth.id),
            None => true,
        })
        .cloned()
        .collect::<Vec<_>>();
    let capture_state = ObserveState {
        capture: CaptureArtifact {
            origin_x: 0.0,
            origin_y: 0.0,
            width: rendered.logical_width,
            height: rendered.logical_height,
            image_width: rendered.image_width,
            image_height: rendered.image_height,
            scale_x: rendered.scale_x,
            scale_y: rendered.scale_y,
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Understudy GUI benchmark".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("bounds".to_string()),
            host_exclusion: HostCaptureExclusionState::default(),
        },
        app_name: Some("Understudy GUI benchmark".to_string()),
    };
    let (session, turn) = live_grounding_benchmark_session().await;

    let mut measurements = Vec::new();
    for truth in &truths {
        let grounding_mode = if truth.difficulty == "complex" {
            "complex"
        } else {
            "single"
        };
        let tool_name = if truth.action == "type" {
            "gui_type"
        } else {
            "gui_click"
        };
        let invocation = gui_invocation(
            session.clone(),
            turn.clone(),
            tool_name,
            serde_json::json!({}),
        );
        let request = GuiTargetRequest {
            app: Some("Understudy GUI benchmark"),
            capture_mode: Some("window"),
            window_selection: None,
            target: truth.target.as_str(),
            location_hint: truth.location_hint.as_deref(),
            scope: truth.scope.as_deref(),
            grounding_mode: Some(grounding_mode),
            action: if truth.action == "type" {
                "type"
            } else {
                "click"
            },
            related_target: None,
            related_scope: None,
            related_location_hint: None,
            related_point: None,
        };

        let started_at = std::time::Instant::now();
        let outcome = default_gui_grounding_provider()
            .ground(
                &invocation,
                request,
                &capture_state,
                &rendered.screenshot_bytes,
            )
            .await;
        let elapsed_ms = started_at.elapsed().as_millis();

        match outcome {
            Ok(Some(resolved)) => {
                let dx = resolved.point.x - truth.point.x;
                let dy = resolved.point.y - truth.point.y;
                let distance_px = (dx * dx + dy * dy).sqrt();
                let inside = point_inside_benchmark_box(&resolved.point, &truth.box_);
                println!(
                    "[codex-grounding-benchmark] case={} found=true inside={} distance={:.1}px elapsed={}ms target={}",
                    truth.id, inside, distance_px, elapsed_ms, truth.target
                );
                measurements.push(GroundingBenchmarkMeasurement {
                    case_id: truth.id.clone(),
                    kind: truth.kind.clone(),
                    prompt_clarity: truth.prompt_clarity.clone(),
                    difficulty: truth.difficulty.clone(),
                    found: true,
                    inside,
                    distance_px,
                    elapsed_ms,
                    error: None,
                });
            }
            Ok(None) => {
                println!(
                    "[codex-grounding-benchmark] case={} found=false inside=false elapsed={}ms target={}",
                    truth.id, elapsed_ms, truth.target
                );
                measurements.push(GroundingBenchmarkMeasurement {
                    case_id: truth.id.clone(),
                    kind: truth.kind.clone(),
                    prompt_clarity: truth.prompt_clarity.clone(),
                    difficulty: truth.difficulty.clone(),
                    found: false,
                    inside: false,
                    distance_px: f64::INFINITY,
                    elapsed_ms,
                    error: None,
                });
            }
            Err(error) => {
                println!(
                    "[codex-grounding-benchmark] case={} error={} elapsed={}ms target={}",
                    truth.id, error, elapsed_ms, truth.target
                );
                measurements.push(GroundingBenchmarkMeasurement {
                    case_id: truth.id.clone(),
                    kind: truth.kind.clone(),
                    prompt_clarity: truth.prompt_clarity.clone(),
                    difficulty: truth.difficulty.clone(),
                    found: false,
                    inside: false,
                    distance_px: f64::INFINITY,
                    elapsed_ms,
                    error: Some(error.to_string()),
                });
            }
        }
    }

    println!(
        "[codex-grounding-benchmark] screenshot={} cases={}",
        rendered.screenshot_path.display(),
        measurements.len()
    );
    summarize_grounding_bucket(&measurements, "explicit", |measurement| {
        measurement.prompt_clarity == "explicit"
    });
    summarize_grounding_bucket(&measurements, "ambiguous", |measurement| {
        measurement.prompt_clarity == "ambiguous"
    });
    summarize_grounding_bucket(&measurements, "complex", |measurement| {
        measurement.difficulty == "complex"
    });
    summarize_grounding_bucket(&measurements, "type", |measurement| {
        measurement.kind == "text_field"
    });

    let total_hit_rate = measurements
        .iter()
        .filter(|measurement| measurement.inside)
        .count() as f64
        / measurements.len() as f64;
    let explicit_measurements: Vec<_> = measurements
        .iter()
        .filter(|measurement| measurement.prompt_clarity == "explicit")
        .collect();
    let ambiguous_measurements: Vec<_> = measurements
        .iter()
        .filter(|measurement| measurement.prompt_clarity == "ambiguous")
        .collect();
    let complex_explicit_measurements: Vec<_> = measurements
        .iter()
        .filter(|measurement| {
            measurement.prompt_clarity == "explicit" && measurement.difficulty == "complex"
        })
        .collect();
    let explicit_hit_rate = explicit_measurements
        .iter()
        .filter(|measurement| measurement.inside)
        .count() as f64
        / explicit_measurements.len() as f64;
    let ambiguous_hit_rate = ambiguous_measurements
        .iter()
        .filter(|measurement| measurement.inside)
        .count() as f64
        / ambiguous_measurements.len() as f64;
    let complex_explicit_hit_rate = complex_explicit_measurements
        .iter()
        .filter(|measurement| measurement.inside)
        .count() as f64
        / complex_explicit_measurements.len() as f64;

    let provider_errors: Vec<_> = measurements
        .iter()
        .filter(|measurement| measurement.error.is_some())
        .map(|measurement| measurement.case_id.clone())
        .collect();
    let explicit_missing_cases: Vec<_> = explicit_measurements
        .iter()
        .filter(|measurement| !measurement.found)
        .map(|measurement| measurement.case_id.clone())
        .collect();
    let point_distance_outliers: Vec<_> = measurements
        .iter()
        .filter_map(|measurement| {
            let truth = truths
                .iter()
                .find(|truth| truth.id == measurement.case_id)?;
            if measurement.found && measurement.distance_px > allowed_point_distance_px(truth) {
                Some(measurement.case_id.clone())
            } else {
                None
            }
        })
        .collect();

    println!(
        "[codex-grounding-benchmark] total={:.2} explicit={:.2} ambiguous={:.2} complex_explicit={:.2}",
        total_hit_rate, explicit_hit_rate, ambiguous_hit_rate, complex_explicit_hit_rate
    );

    let mut failures = Vec::new();
    if !explicit_missing_cases.is_empty() {
        failures.push(format!(
            "missing explicit cases: {}",
            explicit_missing_cases.join(", ")
        ));
    }
    if total_hit_rate < MIN_TOTAL_INSIDE_HIT_RATE {
        failures.push(format!(
            "total inside hit rate {:.2} is below {:.2}",
            total_hit_rate, MIN_TOTAL_INSIDE_HIT_RATE
        ));
    }
    if explicit_hit_rate < MIN_EXPLICIT_INSIDE_HIT_RATE {
        failures.push(format!(
            "explicit inside hit rate {:.2} is below {:.2}",
            explicit_hit_rate, MIN_EXPLICIT_INSIDE_HIT_RATE
        ));
    }
    if ambiguous_hit_rate < MIN_AMBIGUOUS_INSIDE_HIT_RATE {
        failures.push(format!(
            "ambiguous inside hit rate {:.2} is below {:.2}",
            ambiguous_hit_rate, MIN_AMBIGUOUS_INSIDE_HIT_RATE
        ));
    }
    if complex_explicit_hit_rate < MIN_COMPLEX_EXPLICIT_INSIDE_HIT_RATE {
        failures.push(format!(
            "complex explicit hit rate {:.2} is below {:.2}",
            complex_explicit_hit_rate, MIN_COMPLEX_EXPLICIT_INSIDE_HIT_RATE
        ));
    }
    if !provider_errors.is_empty() {
        failures.push(format!("provider errors: {}", provider_errors.join(", ")));
    }
    if !point_distance_outliers.is_empty() {
        failures.push(format!(
            "distance outliers: {}",
            point_distance_outliers.join(", ")
        ));
    }

    assert!(
        failures.is_empty(),
        "grounding benchmark failures:\n{}",
        failures.join("\n")
    );
}
