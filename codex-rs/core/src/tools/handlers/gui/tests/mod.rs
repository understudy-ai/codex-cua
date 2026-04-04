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
use super::grounding::gui_grounding_output_schema;
use super::grounding::image_point_to_display;
use super::grounding::image_rect_to_display;
use super::grounding::prepare_grounding_model_image;
use super::grounding::render_guide_overlay;
use super::grounding::render_validation_overlay;
use super::grounding::request_model_json_from_image;
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

mod benchmark;
mod coordinate_space;
mod drag;
mod grounding;
mod readiness;
mod scroll;
mod smoke_tests;
mod type_and_input;
mod wait;
mod window_selection;

pub(super) fn gui_invocation(
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
