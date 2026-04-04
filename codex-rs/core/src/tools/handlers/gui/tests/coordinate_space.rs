use super::*;

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

            display_index: 1,
            capture_mode: CaptureMode::Window,
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

            display_index: 1,
            capture_mode: CaptureMode::Window,
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
    // Rect that extends past the right edge should be clamped, not rejected.
    let clamped = local_rect_within_state(
        &state,
        &HelperRect {
            x: 750.0,
            y: 20.0,
            width: 100.0,
            height: 80.0,
        },
    )
    .expect("rect with origin inside image should be clamped");
    assert_eq!(clamped.x, 750.0);
    assert_eq!(clamped.width, 50.0); // 800 - 750
    assert_eq!(clamped.height, 80.0);

    // Rect with origin completely outside the image should still be rejected.
    assert!(
        local_rect_within_state(
            &state,
            &HelperRect {
                x: 850.0,
                y: 20.0,
                width: 100.0,
                height: 80.0,
            },
        )
        .is_none()
    );
}

#[test]
fn coordinate_helpers_require_complete_pairs_and_valid_spaces() {
    assert_eq!(
        normalize_coordinate_space(None).expect("default coordinate space"),
        GuiCoordinateSpace::ImagePixels,
    );
    assert_eq!(
        normalize_coordinate_space(Some("display_points")).expect("display space"),
        GuiCoordinateSpace::DisplayPoints,
    );
    assert!(normalize_coordinate_space(Some("screen_pixels")).is_err());

    let point = normalize_optional_coordinate_point(Some(12.0), Some(34.0), "x", "y")
        .expect("coordinate pair")
        .expect("point");
    assert_eq!(point.x, 12.0);
    assert_eq!(point.y, 34.0);
    assert!(normalize_optional_coordinate_point(Some(12.0), None, "x", "y").is_err());
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

            display_index: 1,
            capture_mode: CaptureMode::Window,
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

            display_index: 1,
            capture_mode: CaptureMode::Window,
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
