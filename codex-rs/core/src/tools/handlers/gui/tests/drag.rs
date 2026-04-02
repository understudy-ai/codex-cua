use super::*;

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
