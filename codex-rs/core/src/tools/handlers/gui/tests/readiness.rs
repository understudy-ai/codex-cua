use super::*;

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

#[tokio::test]
async fn gui_observe_targetless_request_skips_image_attachment_without_image_input() {
    let (session, mut turn) = crate::codex::make_session_and_context().await;
    turn.model_info.input_modalities = vec![codex_protocol::openai_models::InputModality::Text];
    let invocation = gui_invocation(
        Arc::new(session),
        Arc::new(turn),
        "gui_observe",
        serde_json::json!({}),
    );

    let attach_image = prepare_gui_observe_request(&invocation, false, None)
        .await
        .expect("targetless gui_observe should remain available without image input");

    assert!(!attach_image);
}

#[tokio::test]
async fn gui_observe_targeted_request_stays_blocked_without_image_input() {
    let (session, mut turn) = crate::codex::make_session_and_context().await;
    turn.model_info.input_modalities = vec![codex_protocol::openai_models::InputModality::Text];
    let invocation = gui_invocation(
        Arc::new(session),
        Arc::new(turn),
        "gui_observe",
        serde_json::json!({
            "target": "Search field",
        }),
    );

    let error = prepare_gui_observe_request(&invocation, true, None)
        .await
        .expect_err("targeted gui_observe should still be blocked without image input");

    assert!(
        error
            .to_string()
            .contains("Keyboard-only tool (gui_key) and targetless gui_type still work.")
            || error.to_string().contains("targetless"),
        "unexpected capability error: {error}"
    );
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
