use std::collections::HashMap;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;

use super::GUI_UNSUPPORTED_MESSAGE;
use super::platform::default_gui_platform;
use super::platform::resolve_gui_platform_tool_capabilities;
use super::supports_image_input;

const GUI_ACCESSIBILITY_REQUIRED_REASON: &str = "Accessibility permission is not granted, so GUI input actions (click, type, scroll, drag, etc.) are unavailable. GUI observation tools (gui_observe) may still work. Grant Accessibility permission in System Settings > Privacy & Security > Accessibility.";
const GUI_GROUNDING_REQUIRED_REASON: &str = "Visual grounding is not configured, so grounding-based GUI actions (click, drag, etc.) are unavailable. Keyboard-only tool (gui_key) and targetless gui_type still work.";
const GUI_NATIVE_HELPER_REQUIRED_REASON: &str = "Native GUI helper is unavailable, so GUI tools cannot run. Verify the helper binary is installed and accessible.";
const GUI_SCREEN_CAPTURE_REQUIRED_REASON: &str = "Screen Recording permission is not granted, so screenshot-based GUI actions are unavailable. Keyboard-only tool (gui_key) still works. Grant Screen Recording permission in System Settings > Privacy & Security > Screen Recording.";
const GUI_SCREEN_CAPTURE_TARGETLESS_ONLY_REASON: &str = "Screen Recording permission is not granted, so this tool only supports targetless usage (omit the `target` parameter). Keyboard-only tool (gui_key) still works.";
const GUI_TARGETLESS_ONLY_REASON: &str = "Visual grounding is not configured, so this tool only supports targetless usage (omit the `target` parameter). It will operate on the current surface or focused control.";

pub(super) const GUI_TOOL_NAMES: [&str; 8] = [
    "gui_observe",
    "gui_click",
    "gui_drag",
    "gui_scroll",
    "gui_type",
    "gui_key",
    "gui_wait",
    "gui_move",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GuiReadinessStatus {
    Ok,
    Warn,
    Error,
    Unsupported,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(super) struct GuiEnvironmentReadinessCheck {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) status: GuiReadinessStatus,
    pub(super) summary: String,
    pub(super) detail: Option<String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(super) struct GuiEnvironmentReadinessSnapshot {
    pub(super) status: &'static str,
    pub(super) checks: Vec<GuiEnvironmentReadinessCheck>,
}

#[derive(Clone, Debug)]
pub(super) struct GuiToolCapability {
    pub(super) enabled: bool,
    pub(super) reason: Option<String>,
    pub(super) targetless_only: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(super) struct GuiRuntimeCapabilitySnapshot {
    pub(super) platform_supported: bool,
    pub(super) grounding_available: bool,
    pub(super) native_helper_available: bool,
    pub(super) screen_capture_available: bool,
    pub(super) input_available: bool,
    pub(super) enabled_tool_names: Vec<&'static str>,
    pub(super) disabled_tool_names: Vec<&'static str>,
    pub(super) tool_availability: HashMap<&'static str, GuiToolCapability>,
}

pub(super) fn resolve_gui_readiness_snapshot() -> GuiEnvironmentReadinessSnapshot {
    default_gui_platform().readiness_snapshot()
}

fn resolve_readiness_check_status(
    snapshot: &GuiEnvironmentReadinessSnapshot,
    check_id: &str,
) -> Option<GuiReadinessStatus> {
    snapshot
        .checks
        .iter()
        .find(|check| check.id == check_id)
        .map(|check| check.status)
}

pub(super) fn resolve_gui_runtime_capabilities(
    grounding_available: bool,
    readiness: &GuiEnvironmentReadinessSnapshot,
    platform_tool_availability: Option<&HashMap<&'static str, GuiToolCapability>>,
) -> GuiRuntimeCapabilitySnapshot {
    let platform_supported = resolve_readiness_check_status(readiness, "platform")
        != Some(GuiReadinessStatus::Unsupported);
    let native_helper_available = platform_supported
        && resolve_readiness_check_status(readiness, "native_helper")
            != Some(GuiReadinessStatus::Error);
    let input_available = native_helper_available
        && resolve_readiness_check_status(readiness, "accessibility")
            != Some(GuiReadinessStatus::Error);
    let screen_capture_available = native_helper_available
        && resolve_readiness_check_status(readiness, "screen_recording")
            != Some(GuiReadinessStatus::Error);

    let mut tool_availability = HashMap::new();
    for tool_name in GUI_TOOL_NAMES {
        let mut capability = platform_tool_availability
            .and_then(|tool_support| tool_support.get(tool_name))
            .cloned()
            .unwrap_or(GuiToolCapability {
                enabled: true,
                reason: None,
                targetless_only: false,
            });
        if !capability.enabled {
            tool_availability.insert(tool_name, capability);
            continue;
        }
        if !platform_supported {
            capability = GuiToolCapability {
                enabled: false,
                reason: Some(GUI_UNSUPPORTED_MESSAGE.to_string()),
                targetless_only: false,
            };
        } else if !native_helper_available {
            capability = GuiToolCapability {
                enabled: false,
                reason: Some(GUI_NATIVE_HELPER_REQUIRED_REASON.to_string()),
                targetless_only: false,
            };
        } else if matches!(
            tool_name,
            "gui_click" | "gui_drag" | "gui_scroll" | "gui_type" | "gui_key" | "gui_move"
        ) && !input_available
        {
            capability = GuiToolCapability {
                enabled: false,
                reason: Some(GUI_ACCESSIBILITY_REQUIRED_REASON.to_string()),
                targetless_only: false,
            };
        } else if matches!(
            tool_name,
            "gui_observe" | "gui_click" | "gui_drag" | "gui_wait"
        ) && !screen_capture_available
        {
            capability = GuiToolCapability {
                enabled: false,
                reason: Some(GUI_SCREEN_CAPTURE_REQUIRED_REASON.to_string()),
                targetless_only: false,
            };
        } else if !screen_capture_available && matches!(tool_name, "gui_scroll" | "gui_type") {
            if !capability.targetless_only {
                capability = GuiToolCapability {
                    enabled: true,
                    reason: Some(GUI_SCREEN_CAPTURE_TARGETLESS_ONLY_REASON.to_string()),
                    targetless_only: true,
                };
            }
        } else if matches!(tool_name, "gui_key" | "gui_move") {
            capability = GuiToolCapability {
                enabled: true,
                reason: capability.reason.clone(),
                targetless_only: capability.targetless_only,
            };
        } else if grounding_available {
            capability = GuiToolCapability {
                enabled: true,
                reason: capability.reason.clone(),
                targetless_only: capability.targetless_only,
            };
        } else if matches!(tool_name, "gui_observe" | "gui_scroll" | "gui_type") {
            if !capability.targetless_only {
                capability = GuiToolCapability {
                    enabled: true,
                    reason: Some(GUI_TARGETLESS_ONLY_REASON.to_string()),
                    targetless_only: true,
                };
            }
        } else if matches!(tool_name, "gui_click" | "gui_drag" | "gui_wait") {
            capability = GuiToolCapability {
                enabled: false,
                reason: Some(GUI_GROUNDING_REQUIRED_REASON.to_string()),
                targetless_only: false,
            };
        }
        tool_availability.insert(tool_name, capability);
    }
    let enabled_tool_names = GUI_TOOL_NAMES
        .into_iter()
        .filter(|tool_name| tool_availability[*tool_name].enabled)
        .collect();
    let disabled_tool_names = GUI_TOOL_NAMES
        .into_iter()
        .filter(|tool_name| !tool_availability[*tool_name].enabled)
        .collect();
    GuiRuntimeCapabilitySnapshot {
        platform_supported,
        grounding_available,
        native_helper_available,
        screen_capture_available,
        input_available,
        enabled_tool_names,
        disabled_tool_names,
        tool_availability,
    }
}

pub(super) fn enforce_gui_tool_capability(
    invocation: &ToolInvocation,
    tool_name: &'static str,
    targeted: bool,
) -> Result<(), FunctionCallError> {
    let readiness = resolve_gui_readiness_snapshot();
    let platform_tool_availability = resolve_gui_platform_tool_capabilities();
    let capabilities = resolve_gui_runtime_capabilities(
        supports_image_input(invocation),
        &readiness,
        Some(&platform_tool_availability),
    );
    let Some(capability) = capabilities.tool_availability.get(tool_name) else {
        return Ok(());
    };
    if !capability.enabled {
        return Err(FunctionCallError::RespondToModel(
            capability
                .reason
                .clone()
                .unwrap_or_else(|| format!("`{tool_name}` is currently unavailable.")),
        ));
    }
    if capability.targetless_only && targeted {
        return Err(FunctionCallError::RespondToModel(
            capability.reason.clone().unwrap_or_else(|| {
                format!("`{tool_name}` currently supports only targetless usage.")
            }),
        ));
    }
    Ok(())
}
