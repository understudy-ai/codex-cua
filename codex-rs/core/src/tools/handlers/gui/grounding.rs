use std::io::Cursor;
use std::time::Duration;

use crate::Prompt;
use crate::client_common::ResponseEvent;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use futures::StreamExt;
use image::DynamicImage;
use image::GenericImageView;
use image::ImageFormat;
use image::Rgba;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::CaptureArtifact;
use super::GroundingBoundingBox;
use super::GroundingModelResponse;
use super::GuiTargetRequest;
use super::HelperPoint;
use super::HelperRect;
use super::ObserveState;
use super::ResolvedTarget;
use super::data_url;
use super::image_point_within_capture;
use super::local_rect_within_state;
use super::normalize_grounding_mode;

const GUI_GROUNDING_SYSTEM_PROMPT: &str = concat!(
    "You are grounding a GUI target inside a screenshot. ",
    "Return JSON only, following the provided schema exactly. ",
    "Resolve the requested target only when it is clearly visible in the screenshot. ",
    "If the target is not confidently visible, return `status` = `not_found`, `found` = false, ",
    "and null coordinates."
);

const GUI_GROUNDING_VALIDATION_SYSTEM_PROMPT: &str = concat!(
    "You are validating a GUI grounding prediction inside a screenshot. ",
    "A highlighted marker indicates the proposed click point and target region. ",
    "Return JSON only, following the provided schema exactly. ",
    "Approve the prediction only when the highlighted point is a good interaction point for the requested target."
);

const GUI_GROUNDING_REFINEMENT_SYSTEM_PROMPT: &str = concat!(
    "You are refining a GUI grounding candidate inside a zoomed crop from the original screenshot. ",
    "Return JSON only, following the provided schema exactly. ",
    "Refine the point and box to the exact actionable or editable surface inside this crop. ",
    "If the crop does not actually contain the requested target, return `status` = `not_found`, `found` = false, and null coordinates."
);

const REFINEMENT_TINY_TARGET_MAX_DIMENSION: f64 = 160.0;
const REFINEMENT_TINY_TARGET_MAX_AREA_FRACTION: f64 = 0.02;
const REFINEMENT_MIN_CROP_WIDTH: u32 = 360;
const REFINEMENT_MIN_CROP_HEIGHT: u32 = 320;
const REFINEMENT_DEFAULT_CANDIDATE_BOX: f64 = 24.0;
const REFINEMENT_MIN_LONGEST_EDGE: f64 = 1200.0;
const REFINEMENT_MAX_SCALE_FACTOR: f64 = 4.0;
const REFINEMENT_MAX_IMAGE_DIMENSION: f64 = 2000.0;
const MODEL_IMAGE_MAX_BYTES: usize = 4_718_592;
const MODEL_IMAGE_MAX_WIDTH: u32 = 2000;
const MODEL_IMAGE_MAX_HEIGHT: u32 = 2000;
const MODEL_IMAGE_DEFAULT_JPEG_QUALITY: u8 = 80;
const MODEL_IMAGE_JPEG_QUALITY_STEPS: [u8; 4] = [85, 70, 55, 40];
const MODEL_IMAGE_SCALE_STEPS: [f64; 5] = [1.0, 0.75, 0.5, 0.35, 0.25];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
struct GroundingValidationResponse {
    status: String,
    approved: bool,
    confidence: Option<f64>,
    reason: Option<String>,
    failure_kind: Option<String>,
    retry_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct RefinementCrop {
    pub(super) image_bytes: Vec<u8>,
    pub(super) offset_x: f64,
    pub(super) offset_y: f64,
    pub(super) crop_width: u32,
    pub(super) crop_height: u32,
    pub(super) model_width: u32,
    pub(super) model_height: u32,
    pub(super) model_scale_x: f64,
    pub(super) model_scale_y: f64,
}

#[derive(Clone, Debug)]
pub(super) struct PreparedGroundingImage {
    pub(super) bytes: Vec<u8>,
    pub(super) mime_type: &'static str,
    pub(super) original_width: u32,
    pub(super) original_height: u32,
    pub(super) working_width: u32,
    pub(super) working_height: u32,
    pub(super) model_width: u32,
    pub(super) model_height: u32,
    pub(super) was_resized: bool,
    pub(super) logical_normalization_applied: bool,
    pub(super) working_to_original_scale_x: f64,
    pub(super) working_to_original_scale_y: f64,
    pub(super) model_to_original_scale_x: f64,
    pub(super) model_to_original_scale_y: f64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct GroundingModelImageConfig {
    pub(super) logical_width: Option<u32>,
    pub(super) logical_height: Option<u32>,
    pub(super) scale_x: Option<f64>,
    pub(super) scale_y: Option<f64>,
    pub(super) allow_logical_normalization: bool,
}

#[derive(Clone, Copy)]
struct ModelInputImage<'a> {
    bytes: &'a [u8],
    mime_type: &'a str,
}

pub(super) fn gui_grounding_provider_name(invocation: &ToolInvocation) -> String {
    format!(
        "{}:{}",
        invocation.turn.config.model_provider_id, invocation.turn.model_info.slug
    )
}

pub(super) fn gui_grounding_output_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": { "type": "string" },
            "found": { "type": "boolean" },
            "confidence": { "type": ["number", "null"] },
            "reason": { "type": ["string", "null"] },
            "coordinate_space": { "type": ["string", "null"] },
            "click_point": {
                "type": ["object", "null"],
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                },
                "required": ["x", "y"],
                "additionalProperties": false
            },
            "bbox": {
                "type": ["object", "null"],
                "properties": {
                    "x1": { "type": "number" },
                    "y1": { "type": "number" },
                    "x2": { "type": "number" },
                    "y2": { "type": "number" }
                },
                "required": ["x1", "y1", "x2", "y2"],
                "additionalProperties": false
            }
        },
        "required": [
            "status",
            "found",
            "confidence",
            "reason",
            "coordinate_space",
            "click_point",
            "bbox"
        ],
        "additionalProperties": false
    })
}

fn gui_grounding_validation_output_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": { "type": "string" },
            "approved": { "type": "boolean" },
            "confidence": { "type": ["number", "null"] },
            "reason": { "type": ["string", "null"] },
            "failure_kind": { "type": ["string", "null"] },
            "retry_hint": { "type": ["string", "null"] }
        },
        "required": [
            "status",
            "approved",
            "confidence",
            "reason",
            "failure_kind",
            "retry_hint"
        ],
        "additionalProperties": false
    })
}

fn format_grounding_action_intent(action: &str) -> &'static str {
    match action {
        "observe" => "identify the requested visible UI target for inspection",
        "wait" => "identify whether the requested visible UI target is present",
        "click" => "identify the best clickable hit target",
        "type" => "identify the editable surface that should receive text",
        "scroll" => "identify the target control or scrollable region",
        "drag_source" => "identify the visible drag source surface",
        "drag_destination" => "identify the visible drop destination surface",
        _ => "identify the requested visible UI target",
    }
}

fn action_specific_grounding_instructions(action: &str) -> &'static [&'static str] {
    match action {
        "click" => &[
            "Resolve the actionable surface that visibly supports the requested click, not a broad region, container, badge strip, or generic panel background.",
            "If you can identify only a broad region or container but not the actionable control itself, return `status` = `not_found` instead of guessing a background point.",
            "For clicks, target the clickable surface near the control center unless the visible affordance suggests a safer interaction point.",
            "For clicking, confirmation and primary-action buttons inside dialogs, sheets, popovers, drawers, side panels, and footers are valid targets even when the surrounding surface also contains labels or fields.",
            "When the scope or location hint mentions a dialog, modal, sheet, drawer, panel, footer, or bottom-right region, prefer the visible actionable control in that region instead of nearby headings, copy, or input fields.",
            "When the target names a visible button label inside a dialog, sheet, popover, drawer, panel, or footer, prefer the labeled button itself over nearby headings, copy, badges, or input fields.",
            "For icon-only controls, match the visible symbol or glyph shape itself and return the individual icon-bearing control rather than the whole toolbar, icon row, or surrounding panel.",
            "For dense toolbar or icon-row controls, use the visible symbol shape, local grouping, and neighboring control order together to distinguish adjacent buttons.",
        ],
        "type" => &[
            "For typing, prefer the editable surface itself instead of its surrounding label or container.",
            "If the UI shows a collapsed control, search affordance, or icon-only affordance that would first reveal or focus the editable field, that visible control is also a valid target when it is the only actionable way to reach the field.",
        ],
        "scroll" => &[
            "For scrolling, target the visible scrollable region or the control that clearly owns scrolling, not a nearby heading or label.",
        ],
        "drag_source" | "drag_destination" => &[
            "For drag actions, visible rows, cards, list items, tree items, and labeled surfaces are valid targets even when there is no dedicated drag handle.",
            "Prefer the actual draggable or droppable surface instead of a broad background region.",
        ],
        _ => &[],
    }
}

fn failure_kind_retry_guidance(failure_kind: &str) -> Option<&'static str> {
    match failure_kind {
        "wrong_region" | "scope_mismatch" => Some(
            "Search a different visible area or panel instead of staying near the previous candidate.",
        ),
        "wrong_control" => Some(
            "Keep the same semantic goal, but choose a different visible control that serves it.",
        ),
        "wrong_point" => Some(
            "Stay on the matched control, but move the click point onto the inner actionable or editable surface.",
        ),
        "state_mismatch" => Some(
            "Re-check the visible control state and pick the candidate whose current state best matches the request.",
        ),
        "partial_visibility" => Some(
            "Only resolve a partially visible target when a safe interaction point is clearly visible inside the visible portion.",
        ),
        _ => None,
    }
}

pub(super) fn build_not_found_retry_notes(
    request: GuiTargetRequest<'_>,
    round: usize,
) -> Vec<String> {
    let mut notes = vec![format!(
        "Round {round} returned not_found. Broaden the search while keeping the same semantic goal and visible scope."
    )];
    notes.push(
        "Match the target by visible meaning, label, icon, state, and nearby context together rather than requiring the exact control type named in the request."
            .to_string(),
    );
    notes.push(
        "Equivalent visible controls may include buttons, icon buttons, toolbar items, tabs, rows, links, toggles, menu items, fields, search boxes, combo boxes, text areas, or editors when they clearly serve the same purpose."
            .to_string(),
    );
    notes.extend(
        action_specific_grounding_instructions(request.action)
            .iter()
            .map(|instruction| instruction.to_string()),
    );
    notes
}

fn append_unique_retry_note(retry_notes: &mut Vec<String>, note: String) {
    if !retry_notes.iter().any(|existing| existing == &note) {
        retry_notes.push(note);
    }
}

fn append_unique_retry_notes<I>(retry_notes: &mut Vec<String>, notes: I)
where
    I: IntoIterator<Item = String>,
{
    for note in notes {
        append_unique_retry_note(retry_notes, note);
    }
}

pub(super) fn should_generate_retry_guide(failure_kind: Option<&str>) -> bool {
    !matches!(failure_kind, Some("wrong_region" | "scope_mismatch"))
}

fn gui_grounding_debug_enabled() -> bool {
    std::env::var("CODEX_GUI_GROUNDING_DEBUG")
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "yes" || normalized == "on"
        })
        .unwrap_or(false)
}

fn emit_gui_grounding_debug(
    request: GuiTargetRequest<'_>,
    grounding_mode: &str,
    round_artifacts: &[JsonValue],
) {
    if !gui_grounding_debug_enabled() {
        return;
    }
    let payload = serde_json::json!({
        "target": request.target,
        "scope": request.scope,
        "location_hint": request.location_hint,
        "action": request.action,
        "grounding_mode": grounding_mode,
        "round_artifacts": round_artifacts,
    });
    eprintln!(
        "[codex-gui-grounding-debug] {}",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| {
            "{\"error\":\"failed to serialize grounding debug payload\"}".to_string()
        })
    );
}

pub(super) fn build_gui_grounding_prompt(
    request: GuiTargetRequest<'_>,
    capture_state: &ObserveState,
    grounding_mode: &str,
    retry_notes: &[String],
    has_guide_image: bool,
) -> String {
    let mut lines = vec![
        format!("action: {}", request.action),
        format!(
            "action_intent: {}",
            format_grounding_action_intent(request.action)
        ),
        format!("grounding_mode: {grounding_mode}"),
        format!("target: {}", request.target),
        format!("capture_mode: {}", capture_state.capture.capture_mode),
        format!(
            "capture_size: {}x{}",
            capture_state.capture.image_width, capture_state.capture.image_height
        ),
    ];
    if let Some(app) = request.app {
        lines.push(format!("app: {app}"));
    }
    if let Some(window_title) = capture_state.capture.window_title.as_deref() {
        lines.push(format!("window_title: {window_title}"));
    }
    if let Some(scope) = request.scope {
        lines.push(format!("scope: {scope}"));
    }
    if let Some(location_hint) = request.location_hint {
        lines.push(format!("location_hint: {location_hint}"));
    }
    if let Some(related_target) = request.related_target {
        lines.push(format!("related_target: {related_target}"));
    }
    if let Some(related_scope) = request.related_scope {
        lines.push(format!("related_scope: {related_scope}"));
    }
    if let Some(related_location_hint) = request.related_location_hint {
        lines.push(format!("related_location_hint: {related_location_hint}"));
    }
    if let Some(related_point) = request.related_point {
        lines.push(format!(
            "related_point_display_pixels: ({}, {})",
            related_point.x.round(),
            related_point.y.round()
        ));
    }
    lines.push("Ground the single best visible UI target in this screenshot.".to_string());
    lines.push(
        "Use only visible screenshot evidence. Do not rely on hidden accessibility labels, DOM ids, or implementation names."
            .to_string(),
    );
    lines.push(
        "Disambiguate similar controls using scope, coarse location, nearby visible text, local grouping, visible state, and relative order."
            .to_string(),
    );
    lines.push(
        "Match subtle or weakly labeled controls by the visible label, symbol, indicator, shape, and surrounding context together."
            .to_string(),
    );
    lines.push(
        "Match the target by visible meaning, label, icon or symbol, state, and surrounding context together; do not require the exact control type named in the request."
            .to_string(),
    );
    lines.push(
        "The requested target may appear as a button, icon button, toolbar item, tab, row, link, toggle, menu item, field, search box, combo box, text area, or editor when those clearly fulfill the same visible intent."
            .to_string(),
    );
    lines.push(
        "Choose the smallest obvious actionable or editable surface, and keep the click point on the visible hit target instead of whitespace, padding, decoration, or generic container background."
            .to_string(),
    );
    lines.push(
        "The bbox must tightly cover the actionable or editable surface itself, not a larger container."
            .to_string(),
    );
    lines.push(
        "When the request refers to nearby text, target the actual control or editable surface rather than the descriptive text alone."
            .to_string(),
    );
    lines.push(
        "If the target has a visible state qualifier such as selected, checked, active, highlighted, or disabled, use that state to disambiguate among similar controls."
            .to_string(),
    );
    lines.push(
        "If a matching control appears disabled or greyed-out and the request does not explicitly ask for a disabled control, prefer an enabled matching control if one exists."
            .to_string(),
    );
    lines.push(
        "If the requested target is only partially visible at the edge of the screenshot, resolve it only when a safe interaction point is clearly visible inside the visible portion; otherwise return `status` = `not_found`."
            .to_string(),
    );
    lines.extend(
        action_specific_grounding_instructions(request.action)
            .iter()
            .map(|instruction| instruction.to_string()),
    );
    if !retry_notes.is_empty() {
        lines.push("Retry context:".to_string());
        lines.extend(retry_notes.iter().map(|line| format!("- {line}")));
    }
    if has_guide_image {
        lines.push(
            "An additional guide image is attached with a red overlay showing the previously rejected candidate."
                .to_string(),
        );
        lines.push(
            "Do not repeat the red marked candidate unless the screenshot clearly contradicts the rejection."
                .to_string(),
        );
    }
    lines.push(
        "Return `status: \"resolved\"` only if the screenshot clearly shows the requested target."
            .to_string(),
    );
    lines.push(
        "Return `status: \"not_found\"`, `found: false`, and null coordinates when the target is not confidently visible."
            .to_string(),
    );
    lines.push(
        "Use `coordinate_space: \"image_pixels\"` and make `click_point` the best interaction point inside the visible target."
            .to_string(),
    );
    lines.push(
        "Make `bbox` a tight visible box around the matched target in screenshot pixels."
            .to_string(),
    );
    lines.push("Respond with JSON only.".to_string());
    lines.join("\n")
}

pub(super) fn build_gui_grounding_validation_prompt(
    request: GuiTargetRequest<'_>,
    capture_state: &ObserveState,
    predicted_point: &HelperPoint,
    predicted_bbox: Option<&GroundingBoundingBox>,
    zoomed_crop_context: bool,
) -> String {
    let mut lines = vec![
        format!("action: {}", request.action),
        "grounding_mode: complex".to_string(),
        format!("target: {}", request.target),
        format!("capture_mode: {}", capture_state.capture.capture_mode),
        format!(
            "capture_size: {}x{}",
            capture_state.capture.image_width, capture_state.capture.image_height
        ),
        format!(
            "predicted_click_point_image_pixels: ({}, {})",
            predicted_point.x.round(),
            predicted_point.y.round()
        ),
    ];
    if let Some(bbox) = predicted_bbox {
        lines.push(format!(
            "predicted_bbox_image_pixels: ({}, {}, {}, {})",
            bbox.x1.round(),
            bbox.y1.round(),
            bbox.x2.round(),
            bbox.y2.round()
        ));
    }
    if let Some(app) = request.app {
        lines.push(format!("app: {app}"));
    }
    if let Some(window_title) = capture_state.capture.window_title.as_deref() {
        lines.push(format!("window_title: {window_title}"));
    }
    if let Some(scope) = request.scope {
        lines.push(format!("scope: {scope}"));
    }
    if let Some(location_hint) = request.location_hint {
        lines.push(format!("location_hint: {location_hint}"));
    }
    if let Some(related_target) = request.related_target {
        lines.push(format!("related_target: {related_target}"));
    }
    if let Some(related_scope) = request.related_scope {
        lines.push(format!("related_scope: {related_scope}"));
    }
    if let Some(related_location_hint) = request.related_location_hint {
        lines.push(format!("related_location_hint: {related_location_hint}"));
    }
    if zoomed_crop_context {
        lines.push(
            "The attached screenshot is a zoomed crop around the candidate from the original screenshot."
                .to_string(),
        );
        lines.push(
            "Judge whether the marked control inside this crop is the correct target from the original request."
                .to_string(),
        );
    }
    lines.push(
        "The attached screenshot includes a red crosshair at the predicted click point and a red box around the predicted target."
            .to_string(),
    );
    lines.push(
        "Return `status: \"approved\"` and `approved: true` only if that highlighted point is a correct interaction point for the requested target."
            .to_string(),
    );
    lines.push(
        "Return `status: \"rejected\"` and `approved: false` if the highlight is incorrect, too ambiguous, or points to the wrong element."
            .to_string(),
    );
    lines.push(
        "Reject if the highlighted action lands on whitespace, padding, decoration, generic container background, or on a neighboring control whose visible evidence does not match the request."
            .to_string(),
    );
    lines.push(
        "If a scope or location hint was provided, the candidate must be inside that scope or region; distinguish matching controls in different panels, rows, toolbars, dialogs, or footers by scope."
            .to_string(),
    );
    lines.push(
        "For subtle, tightly packed, or low-contrast controls, approve only when the marked point sits on the visible hit target itself. Minor offset inside the visible hit area is acceptable only when the control is still clearly the intended one."
            .to_string(),
    );
    lines.push(
        "For weakly worded, paraphrased, or ambiguous requests, approve the marked control when it is the strongest visible semantic match among nearby candidates, even if the iconography is stylized rather than literally labeled."
            .to_string(),
    );
    lines.push(
        "When rejecting, use `failure_kind` to classify the issue as `wrong_region`, `scope_mismatch`, `wrong_control`, `wrong_point`, `state_mismatch`, `partial_visibility`, or `other`."
            .to_string(),
    );
    lines.push(
        "When rejecting, provide a short `retry_hint` describing how the next grounding round should search differently."
            .to_string(),
    );
    lines.push(
        "Keep `reason` terse, at most 10 words. Keep `retry_hint` terse, at most 18 words."
            .to_string(),
    );
    lines.push("Respond with JSON only.".to_string());
    lines.join("\n")
}

pub(super) fn build_gui_grounding_refinement_prompt(
    request: GuiTargetRequest<'_>,
    capture_state: &ObserveState,
    crop: &RefinementCrop,
    prior_point: Option<&HelperPoint>,
    prior_bbox: Option<&GroundingBoundingBox>,
) -> String {
    let mut retry_notes = vec![
        "The provided screenshot is a zoomed crop around a previous candidate from the original image.".to_string(),
    ];
    if let Some(prior_point) = prior_point {
        retry_notes.push(format!(
            "Previous crop-relative point: ({}, {}).",
            prior_point.x.round(),
            prior_point.y.round()
        ));
    }
    if let Some(prior_bbox) = prior_bbox {
        retry_notes.push(format!(
            "Previous crop-relative box: ({}, {}, {}, {}).",
            prior_bbox.x1.round(),
            prior_bbox.y1.round(),
            prior_bbox.x2.round(),
            prior_bbox.y2.round()
        ));
    }
    retry_notes.push(
        "Refine the target inside this crop; if the crop does not actually contain the target, return not_found."
            .to_string(),
    );
    build_gui_grounding_prompt(
        GuiTargetRequest {
            target: request.target,
            scope: request.scope,
            app: request.app,
            location_hint: request.location_hint,
            window_selection: request.window_selection,
            grounding_mode: Some("single"),
            action: request.action,
            capture_mode: request.capture_mode,
            related_target: request.related_target,
            related_scope: request.related_scope,
            related_location_hint: request.related_location_hint,
            related_point: None,
        },
        &ObserveState {
            capture: CaptureArtifact {
                origin_x: crop.offset_x,
                origin_y: crop.offset_y,
                width: crop.crop_width,
                height: crop.crop_height,
                image_width: crop.model_width,
                image_height: crop.model_height,
                scale_x: crop.model_scale_x,
                scale_y: crop.model_scale_y,
                display_index: capture_state.capture.display_index,
                capture_mode: capture_state.capture.capture_mode,
                window_title: capture_state.capture.window_title.clone(),
                window_count: capture_state.capture.window_count,
                window_capture_strategy: capture_state.capture.window_capture_strategy.clone(),
                host_exclusion: capture_state.capture.host_exclusion.clone(),
            },
            app_name: capture_state.app_name.clone(),
        },
        "single",
        &retry_notes,
        false,
    )
}

pub(super) fn extract_grounding_json(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (end > start).then_some(&trimmed[start..=end])
}

pub(super) fn summarize_grounding_response(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 240 {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(240).collect::<String>())
    }
}

pub(super) async fn collect_model_output_text(
    mut stream: crate::ResponseStream,
) -> Result<String, FunctionCallError> {
    let mut result = String::new();
    while let Some(event) = stream.next().await.transpose().map_err(|error| {
        FunctionCallError::RespondToModel(format!("GUI grounding stream failed: {error}"))
    })? {
        match event {
            ResponseEvent::OutputTextDelta(delta) => result.push_str(&delta),
            ResponseEvent::OutputItemDone(item) => {
                if result.is_empty()
                    && let ResponseItem::Message { content, .. } = item
                    && let Some(text) = crate::compact::content_items_to_text(&content)
                {
                    result.push_str(&text);
                }
            }
            ResponseEvent::Completed { .. } => break,
            _ => {}
        }
    }
    Ok(result)
}

async fn request_model_json<T>(
    invocation: &ToolInvocation,
    prompt_text: String,
    image: ModelInputImage<'_>,
    system_prompt: &str,
    output_schema: JsonValue,
    request_label: &str,
) -> Result<(T, JsonValue, String), FunctionCallError>
where
    T: DeserializeOwned,
{
    request_model_json_with_images(
        invocation,
        prompt_text,
        &[image],
        system_prompt,
        output_schema,
        request_label,
    )
    .await
}

async fn request_model_json_with_images<T>(
    invocation: &ToolInvocation,
    prompt_text: String,
    images: &[ModelInputImage<'_>],
    system_prompt: &str,
    output_schema: JsonValue,
    request_label: &str,
) -> Result<(T, JsonValue, String), FunctionCallError>
where
    T: DeserializeOwned,
{
    let mut content = vec![ContentItem::InputText { text: prompt_text }];
    content.extend(images.iter().map(|image| ContentItem::InputImage {
        image_url: data_url(image.bytes, image.mime_type),
    }));
    let prompt = Prompt {
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content,
            end_turn: None,
            phase: None,
        }],
        tools: Vec::new(),
        parallel_tool_calls: false,
        base_instructions: BaseInstructions {
            text: system_prompt.to_string(),
        },
        personality: None,
        output_schema: Some(output_schema),
    };
    let turn_metadata_header = invocation.turn.turn_metadata_state.current_header_value();
    let response_text = {
        let mut last_error: Option<String> = None;
        let mut text: Option<String> = None;
        for attempt in 1..=3 {
            let mut client_session = invocation.session.services.model_client.new_session();
            let stream = client_session
                .stream(
                    &prompt,
                    &invocation.turn.model_info,
                    &invocation.turn.session_telemetry,
                    invocation.turn.reasoning_effort,
                    invocation.turn.reasoning_summary,
                    invocation.turn.config.service_tier,
                    turn_metadata_header.as_deref(),
                )
                .await;
            match stream {
                Ok(stream) => match collect_model_output_text(stream).await {
                    Ok(response_text) => {
                        text = Some(response_text);
                        break;
                    }
                    Err(error) => {
                        last_error = Some(error.to_string());
                    }
                },
                Err(error) => {
                    last_error = Some(format!("{request_label} request failed: {error}"));
                }
            }
            if attempt < 3 {
                tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
        }
        text.ok_or_else(|| {
            FunctionCallError::RespondToModel(last_error.unwrap_or_else(|| {
                format!("{request_label} request failed without an error message")
            }))
        })?
    };
    let json_payload = extract_grounding_json(&response_text).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!(
            "{request_label} response was empty or not JSON: {}",
            summarize_grounding_response(&response_text)
        ))
    })?;
    let raw: JsonValue = serde_json::from_str(json_payload).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "{request_label} response was invalid JSON: {error}. Raw: {}",
            summarize_grounding_response(&response_text)
        ))
    })?;
    let parsed: T = serde_json::from_value(raw.clone()).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "{request_label} response did not match the expected schema: {error}"
        ))
    })?;
    Ok((parsed, raw, response_text))
}

fn draw_marker_pixel(image: &mut image::RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && x < image.width() as i32 && y < image.height() as i32 {
        image.put_pixel(x as u32, y as u32, color);
    }
}

fn draw_rect_outline(image: &mut image::RgbaImage, rect: &HelperRect, color: Rgba<u8>) {
    let left = rect.x.floor() as i32;
    let top = rect.y.floor() as i32;
    let right = (rect.x + rect.width - 1.0).ceil() as i32;
    let bottom = (rect.y + rect.height - 1.0).ceil() as i32;
    for x in left..=right {
        draw_marker_pixel(image, x, top, color);
        draw_marker_pixel(image, x, bottom, color);
    }
    for y in top..=bottom {
        draw_marker_pixel(image, left, y, color);
        draw_marker_pixel(image, right, y, color);
    }
}

fn draw_crosshair(image: &mut image::RgbaImage, point: &HelperPoint, color: Rgba<u8>) {
    let x = point.x.round() as i32;
    let y = point.y.round() as i32;
    for offset in -12..=12 {
        draw_marker_pixel(image, x + offset, y, color);
        draw_marker_pixel(image, x, y + offset, color);
    }
    for offset in -3..=3 {
        draw_marker_pixel(image, x + offset, y + offset, Rgba([255, 255, 255, 255]));
        draw_marker_pixel(image, x + offset, y - offset, Rgba([255, 255, 255, 255]));
    }
}

pub(super) fn render_validation_overlay(
    image_bytes: &[u8],
    predicted_point: &HelperPoint,
    predicted_bbox: Option<&GroundingBoundingBox>,
) -> Result<Vec<u8>, FunctionCallError> {
    let mut image = image::load_from_memory(image_bytes)
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to decode GUI grounding image for validation: {error}"
            ))
        })?
        .into_rgba8();
    let accent = Rgba([255, 48, 48, 255]);
    if let Some(bbox) = predicted_bbox.and_then(grounding_bbox_to_rect) {
        draw_rect_outline(&mut image, &bbox, accent);
    }
    draw_crosshair(&mut image, predicted_point, accent);
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to encode GUI grounding validation image: {error}"
            ))
        })?;
    Ok(encoded.into_inner())
}

pub(super) fn render_guide_overlay(
    image_bytes: &[u8],
    rejected_point: &HelperPoint,
    rejected_bbox: Option<&GroundingBoundingBox>,
) -> Result<Vec<u8>, FunctionCallError> {
    render_validation_overlay(image_bytes, rejected_point, rejected_bbox)
}

pub(super) fn annotate_grounding_raw(
    raw: &mut JsonValue,
    grounding_mode: &str,
    selected_attempt: &str,
    validation_triggered: bool,
    validation_status: &str,
    validation_reason: Option<&str>,
    validation_confidence: Option<f64>,
    rounds_attempted: i64,
) {
    if let JsonValue::Object(fields) = raw {
        fields.insert(
            "grounding_mode_requested".to_string(),
            JsonValue::String(grounding_mode.to_string()),
        );
        fields.insert(
            "grounding_mode_effective".to_string(),
            JsonValue::String(grounding_mode.to_string()),
        );
        fields.insert(
            "selected_attempt".to_string(),
            JsonValue::String(selected_attempt.to_string()),
        );
        fields.insert(
            "grounding_validation_triggered".to_string(),
            JsonValue::Bool(validation_triggered),
        );
        fields.insert(
            "grounding_rounds_attempted".to_string(),
            JsonValue::Number(rounds_attempted.into()),
        );
        let mut validation = serde_json::Map::new();
        validation.insert(
            "status".to_string(),
            JsonValue::String(validation_status.to_string()),
        );
        validation.insert(
            "reason".to_string(),
            validation_reason
                .map(|reason| JsonValue::String(reason.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        validation.insert(
            "confidence".to_string(),
            validation_confidence
                .and_then(serde_json::Number::from_f64)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
        );
        fields.insert("validation".to_string(), JsonValue::Object(validation));
    }
}

pub(super) fn annotate_grounding_round_artifacts(
    raw: &mut JsonValue,
    round_artifacts: &[JsonValue],
) {
    if let JsonValue::Object(fields) = raw {
        fields.insert(
            "grounding_round_artifacts".to_string(),
            JsonValue::Array(round_artifacts.to_vec()),
        );
    }
}

fn annotate_prepared_grounding_image_raw(raw: &mut JsonValue, prepared: &PreparedGroundingImage) {
    if let JsonValue::Object(fields) = raw {
        fields.insert(
            "grounding_model_image".to_string(),
            prepared_grounding_image_metadata(prepared),
        );
    }
}

fn prepared_grounding_image_metadata(prepared: &PreparedGroundingImage) -> JsonValue {
    serde_json::json!({
        "mime_type": prepared.mime_type,
        "original_width": prepared.original_width,
        "original_height": prepared.original_height,
        "working_width": prepared.working_width,
        "working_height": prepared.working_height,
        "model_width": prepared.model_width,
        "model_height": prepared.model_height,
        "was_resized": prepared.was_resized,
        "logical_normalization_applied": prepared.logical_normalization_applied,
        "working_to_original_scale_x": prepared.working_to_original_scale_x,
        "working_to_original_scale_y": prepared.working_to_original_scale_y,
        "model_to_original_scale_x": prepared.model_to_original_scale_x,
        "model_to_original_scale_y": prepared.model_to_original_scale_y,
        "byte_length": prepared.bytes.len(),
    })
}

fn refinement_crop_metadata(crop: &RefinementCrop) -> JsonValue {
    serde_json::json!({
        "crop": {
            "x": crop.offset_x,
            "y": crop.offset_y,
            "width": crop.crop_width,
            "height": crop.crop_height,
        },
        "model_image": {
            "width": crop.model_width,
            "height": crop.model_height,
            "scale_x": crop.model_scale_x,
            "scale_y": crop.model_scale_y,
            "byte_length": crop.image_bytes.len(),
            "mime_type": "image/png",
        }
    })
}

fn predictor_outcome_metadata(
    decision: &GroundingModelResponse,
    point: Option<&HelperPoint>,
    bbox: Option<&GroundingBoundingBox>,
) -> JsonValue {
    serde_json::json!({
        "status": decision.status,
        "found": decision.found,
        "confidence": decision.confidence,
        "reason": decision.reason,
        "coordinate_space": decision.coordinate_space,
        "point": point.map(|point| serde_json::json!({ "x": point.x, "y": point.y })),
        "bbox": bbox.map(|bbox| serde_json::json!({
            "x1": bbox.x1,
            "y1": bbox.y1,
            "x2": bbox.x2,
            "y2": bbox.y2,
        })),
    })
}

fn normalize_preferred_dimension(
    preferred_dimension: Option<u32>,
    original_dimension: u32,
    scale_hint: Option<f64>,
) -> Option<u32> {
    if let Some(preferred_dimension) = preferred_dimension.filter(|value| *value > 0) {
        return Some(preferred_dimension.min(original_dimension).max(1));
    }
    if let Some(scale_hint) = scale_hint.filter(|value| value.is_finite() && *value > 1.0) {
        return Some(
            ((original_dimension as f64) / scale_hint)
                .round()
                .clamp(1.0, original_dimension as f64) as u32,
        );
    }
    None
}

fn resolve_working_dimensions(
    original_width: u32,
    original_height: u32,
    config: GroundingModelImageConfig,
) -> (u32, u32, bool) {
    if !config.allow_logical_normalization || original_width == 0 || original_height == 0 {
        return (original_width, original_height, false);
    }
    let original_aspect = original_width as f64 / original_height as f64;
    let mut preferred_width =
        normalize_preferred_dimension(config.logical_width, original_width, config.scale_x);
    let mut preferred_height =
        normalize_preferred_dimension(config.logical_height, original_height, config.scale_y);

    if preferred_width.is_some() && preferred_height.is_none() {
        preferred_height = Some(
            ((preferred_width.unwrap() as f64) / original_aspect)
                .round()
                .clamp(1.0, original_height as f64) as u32,
        );
    }
    if preferred_height.is_some() && preferred_width.is_none() {
        preferred_width = Some(
            ((preferred_height.unwrap() as f64) * original_aspect)
                .round()
                .clamp(1.0, original_width as f64) as u32,
        );
    }
    if let (Some(width), Some(height)) = (preferred_width, preferred_height) {
        let preferred_aspect = width as f64 / height as f64;
        if (preferred_aspect - original_aspect).abs() > 0.02 {
            let height_from_width = ((width as f64) / original_aspect)
                .round()
                .clamp(1.0, original_height as f64) as u32;
            let width_from_height = ((height as f64) * original_aspect)
                .round()
                .clamp(1.0, original_width as f64) as u32;
            if (height_from_width as i64 - height as i64).abs()
                <= (width_from_height as i64 - width as i64).abs()
            {
                preferred_height = Some(height_from_width);
            } else {
                preferred_width = Some(width_from_height);
            }
        }
    }

    let working_width = preferred_width.unwrap_or(original_width);
    let working_height = preferred_height.unwrap_or(original_height);
    (
        working_width,
        working_height,
        working_width != original_width || working_height != original_height,
    )
}

fn constrain_model_dimensions(width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width, height);
    }
    let mut target_width = width;
    let mut target_height = height;
    if target_width > MODEL_IMAGE_MAX_WIDTH {
        target_height = (((target_height as f64) * (MODEL_IMAGE_MAX_WIDTH as f64))
            / (target_width as f64))
            .round()
            .max(1.0) as u32;
        target_width = MODEL_IMAGE_MAX_WIDTH;
    }
    if target_height > MODEL_IMAGE_MAX_HEIGHT {
        target_width = (((target_width as f64) * (MODEL_IMAGE_MAX_HEIGHT as f64))
            / (target_height as f64))
            .round()
            .max(1.0) as u32;
        target_height = MODEL_IMAGE_MAX_HEIGHT;
    }
    (target_width.max(1), target_height.max(1))
}

fn encode_image_to_jpeg(
    image: &DynamicImage,
    quality: u8,
    label: &str,
) -> Result<Vec<u8>, FunctionCallError> {
    let mut encoded = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality);
    encoder.encode_image(image).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to encode GUI grounding {label} as jpeg: {error}"
        ))
    })?;
    Ok(encoded.into_inner())
}

fn resize_exact_if_needed(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    if image.width() == width && image.height() == height {
        image.clone()
    } else {
        image.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
    }
}

pub(super) fn prepare_grounding_model_image(
    image_bytes: &[u8],
    config: GroundingModelImageConfig,
    label: &str,
) -> Result<PreparedGroundingImage, FunctionCallError> {
    let original = image::load_from_memory(image_bytes).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to decode GUI grounding {label}: {error}"
        ))
    })?;
    let original_width = original.width();
    let original_height = original.height();
    if original_width == 0 || original_height == 0 {
        return Err(FunctionCallError::RespondToModel(format!(
            "GUI grounding {label} had empty dimensions"
        )));
    }

    let (working_width, working_height, logical_normalization_applied) =
        resolve_working_dimensions(original_width, original_height, config);
    let working = resize_exact_if_needed(&original, working_width, working_height);
    let (initial_model_width, initial_model_height) =
        constrain_model_dimensions(working_width, working_height);

    let evaluate_candidate = |width: u32,
                              height: u32,
                              jpeg_quality: u8|
     -> Result<Option<(Vec<u8>, &'static str)>, FunctionCallError> {
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let candidate = resize_exact_if_needed(&working, width, height);
        let png = encode_image_to_png(&candidate, label)?;
        let jpeg = encode_image_to_jpeg(&candidate, jpeg_quality, label)?;
        let preferred = if png.len() <= MODEL_IMAGE_MAX_BYTES {
            (png, "image/png")
        } else if jpeg.len() <= MODEL_IMAGE_MAX_BYTES {
            (jpeg, "image/jpeg")
        } else if png.len() <= jpeg.len() {
            (png, "image/png")
        } else {
            (jpeg, "image/jpeg")
        };
        Ok(Some(preferred))
    };

    let (mut best_bytes, mut best_mime_type) = evaluate_candidate(
        initial_model_width,
        initial_model_height,
        MODEL_IMAGE_DEFAULT_JPEG_QUALITY,
    )?
    .ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "GUI grounding image preparation produced an empty candidate".to_string(),
        )
    })?;
    let mut final_width = initial_model_width;
    let mut final_height = initial_model_height;
    let mut found_within_limit = best_bytes.len() <= MODEL_IMAGE_MAX_BYTES;

    for scale in MODEL_IMAGE_SCALE_STEPS {
        let candidate_width = ((initial_model_width as f64) * scale)
            .round()
            .clamp(1.0, initial_model_width as f64) as u32;
        let candidate_height = ((initial_model_height as f64) * scale)
            .round()
            .clamp(1.0, initial_model_height as f64) as u32;
        if candidate_width < 100 || candidate_height < 100 {
            break;
        }
        for quality in MODEL_IMAGE_JPEG_QUALITY_STEPS {
            if let Some((candidate_bytes, candidate_mime_type)) =
                evaluate_candidate(candidate_width, candidate_height, quality)?
            {
                let should_replace = if found_within_limit {
                    candidate_bytes.len() <= MODEL_IMAGE_MAX_BYTES
                        && candidate_bytes.len() < best_bytes.len()
                } else if candidate_bytes.len() <= MODEL_IMAGE_MAX_BYTES {
                    found_within_limit = true;
                    true
                } else {
                    candidate_bytes.len() < best_bytes.len()
                };
                if should_replace {
                    best_bytes = candidate_bytes;
                    best_mime_type = candidate_mime_type;
                    final_width = candidate_width;
                    final_height = candidate_height;
                }
            }
            if found_within_limit && best_bytes.len() <= MODEL_IMAGE_MAX_BYTES {
                break;
            }
        }
        if found_within_limit && best_bytes.len() <= MODEL_IMAGE_MAX_BYTES {
            break;
        }
    }

    let working_to_original_scale_x = if working_width > 0 {
        original_width as f64 / working_width as f64
    } else {
        1.0
    };
    let working_to_original_scale_y = if working_height > 0 {
        original_height as f64 / working_height as f64
    } else {
        1.0
    };
    let model_to_working_scale_x = if final_width > 0 {
        working_width as f64 / final_width as f64
    } else {
        1.0
    };
    let model_to_working_scale_y = if final_height > 0 {
        working_height as f64 / final_height as f64
    } else {
        1.0
    };

    Ok(PreparedGroundingImage {
        bytes: best_bytes,
        mime_type: best_mime_type,
        original_width,
        original_height,
        working_width,
        working_height,
        model_width: final_width,
        model_height: final_height,
        was_resized: logical_normalization_applied
            || final_width != original_width
            || final_height != original_height,
        logical_normalization_applied,
        working_to_original_scale_x,
        working_to_original_scale_y,
        model_to_original_scale_x: model_to_working_scale_x * working_to_original_scale_x,
        model_to_original_scale_y: model_to_working_scale_y * working_to_original_scale_y,
    })
}

pub(super) fn grounding_bbox_to_rect(bbox: &GroundingBoundingBox) -> Option<HelperRect> {
    let width = bbox.x2 - bbox.x1;
    let height = bbox.y2 - bbox.y1;
    if !bbox.x1.is_finite()
        || !bbox.y1.is_finite()
        || !bbox.x2.is_finite()
        || !bbox.y2.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    Some(HelperRect {
        x: bbox.x1,
        y: bbox.y1,
        width,
        height,
    })
}

fn fallback_candidate_rect(point: &HelperPoint) -> HelperRect {
    HelperRect {
        x: (point.x - (REFINEMENT_DEFAULT_CANDIDATE_BOX / 2.0)).max(0.0),
        y: (point.y - (REFINEMENT_DEFAULT_CANDIDATE_BOX / 2.0)).max(0.0),
        width: REFINEMENT_DEFAULT_CANDIDATE_BOX,
        height: REFINEMENT_DEFAULT_CANDIDATE_BOX,
    }
}

pub(super) fn should_use_high_resolution_refinement(
    capture_state: &ObserveState,
    point: &HelperPoint,
    bbox: Option<&GroundingBoundingBox>,
) -> bool {
    let candidate_box = bbox
        .and_then(grounding_bbox_to_rect)
        .unwrap_or_else(|| fallback_candidate_rect(point));
    let max_dimension = candidate_box.width.max(candidate_box.height);
    let image_area =
        (capture_state.capture.image_width as f64) * (capture_state.capture.image_height as f64);
    let box_area = candidate_box.width * candidate_box.height;
    max_dimension <= REFINEMENT_TINY_TARGET_MAX_DIMENSION
        || (image_area > 0.0 && (box_area / image_area) <= REFINEMENT_TINY_TARGET_MAX_AREA_FRACTION)
}

fn encode_image_to_png(
    image: &image::DynamicImage,
    label: &str,
) -> Result<Vec<u8>, FunctionCallError> {
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to encode GUI grounding {label}: {error}"
            ))
        })?;
    Ok(encoded.into_inner())
}

pub(super) fn create_refinement_crop(
    image_bytes: &[u8],
    point: &HelperPoint,
    bbox: Option<&GroundingBoundingBox>,
) -> Result<Option<RefinementCrop>, FunctionCallError> {
    let image = image::load_from_memory(image_bytes).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to decode GUI grounding image for refinement: {error}"
        ))
    })?;
    let (image_width, image_height) = image.dimensions();
    if image_width == 0 || image_height == 0 {
        return Ok(None);
    }

    let candidate_box = bbox
        .and_then(grounding_bbox_to_rect)
        .unwrap_or_else(|| fallback_candidate_rect(point));
    let crop_width = ((candidate_box.width * 5.0)
        .round()
        .max((candidate_box.height * 6.0).round())
        .max(REFINEMENT_MIN_CROP_WIDTH as f64))
    .min(image_width as f64) as u32;
    let crop_height = ((candidate_box.height * 5.0)
        .round()
        .max((candidate_box.width * 4.0).round())
        .max(REFINEMENT_MIN_CROP_HEIGHT as f64))
    .min(image_height as f64) as u32;
    if crop_width == 0 || crop_height == 0 {
        return Ok(None);
    }

    let center_x = point.x.round();
    let center_y = point.y.round();
    let left = (center_x - (crop_width as f64 / 2.0))
        .clamp(0.0, (image_width.saturating_sub(crop_width)) as f64)
        .round() as u32;
    let top = (center_y - (crop_height as f64 / 2.0))
        .clamp(0.0, (image_height.saturating_sub(crop_height)) as f64)
        .round() as u32;

    let cropped = image.crop_imm(left, top, crop_width, crop_height);
    let longest_edge = crop_width.max(crop_height) as f64;
    let desired_scale = if longest_edge > 0.0 && longest_edge < REFINEMENT_MIN_LONGEST_EDGE {
        (REFINEMENT_MIN_LONGEST_EDGE / longest_edge)
            .min(REFINEMENT_MAX_SCALE_FACTOR)
            .min(REFINEMENT_MAX_IMAGE_DIMENSION / longest_edge)
    } else {
        1.0
    };
    let model_width = ((crop_width as f64) * desired_scale)
        .round()
        .clamp(1.0, REFINEMENT_MAX_IMAGE_DIMENSION) as u32;
    let model_height = ((crop_height as f64) * desired_scale)
        .round()
        .clamp(1.0, REFINEMENT_MAX_IMAGE_DIMENSION) as u32;
    let prepared = if model_width != crop_width || model_height != crop_height {
        cropped.resize_exact(
            model_width,
            model_height,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        cropped
    };
    let image_bytes = encode_image_to_png(&prepared, "refinement image")?;

    Ok(Some(RefinementCrop {
        image_bytes,
        offset_x: left as f64,
        offset_y: top as f64,
        crop_width,
        crop_height,
        model_width,
        model_height,
        model_scale_x: model_width as f64 / crop_width as f64,
        model_scale_y: model_height as f64 / crop_height as f64,
    }))
}

pub(super) fn translate_refinement_point_to_original(
    crop: &RefinementCrop,
    point: &HelperPoint,
) -> HelperPoint {
    HelperPoint {
        x: crop.offset_x + (point.x / crop.model_scale_x),
        y: crop.offset_y + (point.y / crop.model_scale_y),
    }
}

fn translate_refinement_bbox_to_original(
    crop: &RefinementCrop,
    bbox: &GroundingBoundingBox,
) -> GroundingBoundingBox {
    GroundingBoundingBox {
        x1: crop.offset_x + (bbox.x1 / crop.model_scale_x),
        y1: crop.offset_y + (bbox.y1 / crop.model_scale_y),
        x2: crop.offset_x + (bbox.x2 / crop.model_scale_x),
        y2: crop.offset_y + (bbox.y2 / crop.model_scale_y),
    }
}

pub(super) fn translate_model_point_to_original(
    prepared: &PreparedGroundingImage,
    point: &HelperPoint,
) -> HelperPoint {
    HelperPoint {
        x: point.x * prepared.model_to_original_scale_x,
        y: point.y * prepared.model_to_original_scale_y,
    }
}

fn translate_model_bbox_to_original(
    prepared: &PreparedGroundingImage,
    bbox: &GroundingBoundingBox,
) -> GroundingBoundingBox {
    GroundingBoundingBox {
        x1: bbox.x1 * prepared.model_to_original_scale_x,
        y1: bbox.y1 * prepared.model_to_original_scale_y,
        x2: bbox.x2 * prepared.model_to_original_scale_x,
        y2: bbox.y2 * prepared.model_to_original_scale_y,
    }
}

pub(super) fn translate_original_point_to_refinement(
    crop: &RefinementCrop,
    point: &HelperPoint,
) -> HelperPoint {
    HelperPoint {
        x: (point.x - crop.offset_x) * crop.model_scale_x,
        y: (point.y - crop.offset_y) * crop.model_scale_y,
    }
}

fn translate_original_bbox_to_refinement(
    crop: &RefinementCrop,
    bbox: &GroundingBoundingBox,
) -> GroundingBoundingBox {
    GroundingBoundingBox {
        x1: (bbox.x1 - crop.offset_x) * crop.model_scale_x,
        y1: (bbox.y1 - crop.offset_y) * crop.model_scale_y,
        x2: (bbox.x2 - crop.offset_x) * crop.model_scale_x,
        y2: (bbox.y2 - crop.offset_y) * crop.model_scale_y,
    }
}

pub(super) fn translate_original_point_to_model(
    prepared: &PreparedGroundingImage,
    point: &HelperPoint,
) -> HelperPoint {
    HelperPoint {
        x: point.x / prepared.model_to_original_scale_x,
        y: point.y / prepared.model_to_original_scale_y,
    }
}

fn translate_original_bbox_to_model(
    prepared: &PreparedGroundingImage,
    bbox: &GroundingBoundingBox,
) -> GroundingBoundingBox {
    GroundingBoundingBox {
        x1: bbox.x1 / prepared.model_to_original_scale_x,
        y1: bbox.y1 / prepared.model_to_original_scale_y,
        x2: bbox.x2 / prepared.model_to_original_scale_x,
        y2: bbox.y2 / prepared.model_to_original_scale_y,
    }
}

pub(super) fn image_point_to_display(state: &ObserveState, point: &HelperPoint) -> HelperPoint {
    let scale_x = if state.capture.scale_x.is_finite() && state.capture.scale_x > 0.0 {
        state.capture.scale_x
    } else {
        1.0
    };
    let scale_y = if state.capture.scale_y.is_finite() && state.capture.scale_y > 0.0 {
        state.capture.scale_y
    } else {
        1.0
    };
    HelperPoint {
        x: state.capture.origin_x + (point.x / scale_x),
        y: state.capture.origin_y + (point.y / scale_y),
    }
}

pub(super) fn image_rect_to_display(state: &ObserveState, rect: &HelperRect) -> HelperRect {
    let scale_x = if state.capture.scale_x.is_finite() && state.capture.scale_x > 0.0 {
        state.capture.scale_x
    } else {
        1.0
    };
    let scale_y = if state.capture.scale_y.is_finite() && state.capture.scale_y > 0.0 {
        state.capture.scale_y
    } else {
        1.0
    };
    HelperRect {
        x: state.capture.origin_x + (rect.x / scale_x),
        y: state.capture.origin_y + (rect.y / scale_y),
        width: rect.width / scale_x,
        height: rect.height / scale_y,
    }
}

pub(super) async fn resolve_grounded_target(
    invocation: &ToolInvocation,
    request: GuiTargetRequest<'_>,
    capture_state: &ObserveState,
    image_bytes: &[u8],
) -> Result<Option<ResolvedTarget>, FunctionCallError> {
    let grounding_mode = normalize_grounding_mode(request.grounding_mode, request.action)?;
    let max_rounds = if grounding_mode == "complex" { 3 } else { 2 };
    let prepared_grounding_image = prepare_grounding_model_image(
        image_bytes,
        GroundingModelImageConfig {
            logical_width: Some(capture_state.capture.width),
            logical_height: Some(capture_state.capture.height),
            scale_x: Some(capture_state.capture.scale_x),
            scale_y: Some(capture_state.capture.scale_y),
            allow_logical_normalization: true,
        },
        "grounding image",
    )?;
    let mut model_capture_state = capture_state.clone();
    model_capture_state.capture.image_width = prepared_grounding_image.model_width;
    model_capture_state.capture.image_height = prepared_grounding_image.model_height;
    let mut retry_notes: Vec<String> = Vec::new();
    let mut guide_image: Option<Vec<u8>> = None;
    let mut round_artifacts: Vec<JsonValue> = Vec::new();
    let mut selected_round = 1_i64;
    let mut final_decision: Option<GroundingModelResponse> = None;
    let mut final_raw: Option<JsonValue> = None;
    let mut final_validation: Option<(GroundingValidationResponse, JsonValue, String, usize)> =
        None;
    let mut validation_zoom_context = false;
    let mut validation_image_bytes_override: Option<Vec<u8>> = None;
    let mut validation_state_override: Option<ObserveState> = None;
    let mut validation_point_override: Option<HelperPoint> = None;
    let mut validation_bbox_override: Option<GroundingBoundingBox> = None;

    for round in 1..=max_rounds {
        let mut grounding_images = vec![ModelInputImage {
            bytes: &prepared_grounding_image.bytes,
            mime_type: prepared_grounding_image.mime_type,
        }];
        if let Some(guide_image_bytes) = guide_image.as_deref() {
            grounding_images.push(ModelInputImage {
                bytes: guide_image_bytes,
                mime_type: "image/png",
            });
        }
        let (decision, raw, _) = request_model_json_with_images::<GroundingModelResponse>(
            invocation,
            build_gui_grounding_prompt(
                request,
                &model_capture_state,
                grounding_mode,
                &retry_notes,
                guide_image.is_some(),
            ),
            &grounding_images,
            GUI_GROUNDING_SYSTEM_PROMPT,
            gui_grounding_output_schema(),
            "GUI grounding",
        )
        .await?;
        let mut round_artifact = serde_json::json!({
            "round": round,
            "guide_image_attached": guide_image.is_some(),
            "model_image": prepared_grounding_image_metadata(&prepared_grounding_image),
            "retry_notes_before_round": retry_notes.clone(),
            "predictor": {
                "raw": raw.clone(),
                "outcome": predictor_outcome_metadata(&decision, decision.click_point.as_ref(), decision.bbox.as_ref()),
            },
        });
        if decision.status == "not_found" || !decision.found {
            if let Some(reason) = decision.reason.as_deref() {
                append_unique_retry_note(
                    &mut retry_notes,
                    format!("Round {round} predictor rationale: {reason}"),
                );
            }
            if let JsonValue::Object(fields) = &mut round_artifact {
                fields.insert(
                    "terminal_state".to_string(),
                    JsonValue::String(if round < max_rounds {
                        "predictor_not_found_retry".to_string()
                    } else {
                        "predictor_not_found_final".to_string()
                    }),
                );
                fields.insert(
                    "retry_notes_after_round".to_string(),
                    JsonValue::Array(retry_notes.iter().cloned().map(JsonValue::String).collect()),
                );
            }
            round_artifacts.push(round_artifact);
            if round < max_rounds {
                append_unique_retry_notes(
                    &mut retry_notes,
                    build_not_found_retry_notes(request, round),
                );
                guide_image = None;
                continue;
            }
            emit_gui_grounding_debug(request, grounding_mode, &round_artifacts);
            return Ok(None);
        }
        if decision.status != "resolved" {
            return Err(FunctionCallError::RespondToModel(format!(
                "GUI grounding returned unsupported status `{}`",
                decision.status
            )));
        }
        let coordinate_space = decision
            .coordinate_space
            .as_deref()
            .unwrap_or("image_pixels");
        if coordinate_space != "image_pixels" {
            return Err(FunctionCallError::RespondToModel(format!(
                "GUI grounding returned unsupported coordinate space `{coordinate_space}`"
            )));
        }
        let model_image_point = decision.click_point.clone().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "GUI grounding resolved a target without a click_point".to_string(),
            )
        })?;
        let mut candidate_decision = decision.clone();
        let mut candidate_raw = raw.clone();
        let image_point =
            translate_model_point_to_original(&prepared_grounding_image, &model_image_point);
        candidate_decision.click_point = Some(image_point.clone());
        candidate_decision.bbox = decision
            .bbox
            .as_ref()
            .map(|bbox| translate_model_bbox_to_original(&prepared_grounding_image, bbox));
        annotate_prepared_grounding_image_raw(&mut candidate_raw, &prepared_grounding_image);
        if grounding_mode == "complex"
            && should_use_high_resolution_refinement(
                capture_state,
                &image_point,
                candidate_decision.bbox.as_ref(),
            )
            && let Some(refinement_crop) =
                create_refinement_crop(image_bytes, &image_point, candidate_decision.bbox.as_ref())?
        {
            let prior_point =
                translate_original_point_to_refinement(&refinement_crop, &image_point);
            let prior_bbox = candidate_decision
                .bbox
                .as_ref()
                .map(|bbox| translate_original_bbox_to_refinement(&refinement_crop, bbox));
            let (refinement_decision, refinement_raw, _) =
                request_model_json::<GroundingModelResponse>(
                    invocation,
                    build_gui_grounding_refinement_prompt(
                        request,
                        capture_state,
                        &refinement_crop,
                        Some(&prior_point),
                        prior_bbox.as_ref(),
                    ),
                    ModelInputImage {
                        bytes: &refinement_crop.image_bytes,
                        mime_type: "image/png",
                    },
                    GUI_GROUNDING_REFINEMENT_SYSTEM_PROMPT,
                    gui_grounding_output_schema(),
                    "GUI grounding refinement",
                )
                .await?;
            if let JsonValue::Object(fields) = &mut round_artifact {
                fields.insert(
                    "refinement".to_string(),
                    serde_json::json!({
                        "attempted": true,
                        "raw": refinement_raw.clone(),
                        "crop": refinement_crop_metadata(&refinement_crop),
                        "outcome": predictor_outcome_metadata(
                            &refinement_decision,
                            refinement_decision.click_point.as_ref(),
                            refinement_decision.bbox.as_ref(),
                        ),
                    }),
                );
            }
            if refinement_decision.status == "resolved"
                && refinement_decision.found
                && refinement_decision
                    .coordinate_space
                    .as_deref()
                    .unwrap_or("image_pixels")
                    == "image_pixels"
                && let Some(refined_point) = refinement_decision.click_point.as_ref()
            {
                let refined_point =
                    translate_refinement_point_to_original(&refinement_crop, refined_point);
                candidate_decision.click_point = Some(refined_point);
                candidate_decision.bbox = refinement_decision
                    .bbox
                    .as_ref()
                    .map(|bbox| translate_refinement_bbox_to_original(&refinement_crop, bbox));
                if let JsonValue::Object(fields) = &mut candidate_raw {
                    fields.insert("initial_prediction_raw".to_string(), raw.clone());
                    fields.insert("refinement_raw".to_string(), refinement_raw);
                    fields.insert(
                        "grounding_refinement_applied".to_string(),
                        JsonValue::Bool(true),
                    );
                    fields.insert(
                        "grounding_refinement_crop".to_string(),
                        serde_json::json!({
                            "x": refinement_crop.offset_x,
                            "y": refinement_crop.offset_y,
                            "width": refinement_crop.crop_width,
                            "height": refinement_crop.crop_height,
                        }),
                    );
                    fields.insert(
                        "grounding_refinement_model_image".to_string(),
                        serde_json::json!({
                            "width": refinement_crop.model_width,
                            "height": refinement_crop.model_height,
                            "scale_x": refinement_crop.model_scale_x,
                            "scale_y": refinement_crop.model_scale_y,
                        }),
                    );
                }
                if let JsonValue::Object(fields) = &mut round_artifact {
                    fields.insert("refinement_applied".to_string(), JsonValue::Bool(true));
                }
                validation_zoom_context = true;
                validation_image_bytes_override = Some(refinement_crop.image_bytes.clone());
                validation_state_override = Some(ObserveState {
                    capture: CaptureArtifact {
                        origin_x: refinement_crop.offset_x,
                        origin_y: refinement_crop.offset_y,
                        width: refinement_crop.crop_width,
                        height: refinement_crop.crop_height,
                        image_width: refinement_crop.model_width,
                        image_height: refinement_crop.model_height,
                        scale_x: refinement_crop.model_scale_x,
                        scale_y: refinement_crop.model_scale_y,
                        display_index: capture_state.capture.display_index,
                        capture_mode: capture_state.capture.capture_mode,
                        window_title: capture_state.capture.window_title.clone(),
                        window_count: capture_state.capture.window_count,
                        window_capture_strategy: capture_state
                            .capture
                            .window_capture_strategy
                            .clone(),
                        host_exclusion: capture_state.capture.host_exclusion.clone(),
                    },
                    app_name: capture_state.app_name.clone(),
                });
                validation_point_override = candidate_decision
                    .click_point
                    .as_ref()
                    .map(|point| translate_original_point_to_refinement(&refinement_crop, point));
                validation_bbox_override = candidate_decision
                    .bbox
                    .as_ref()
                    .map(|bbox| translate_original_bbox_to_refinement(&refinement_crop, bbox));
            }
        } else if let JsonValue::Object(fields) = &mut round_artifact {
            fields.insert(
                "refinement".to_string(),
                serde_json::json!({
                    "attempted": false,
                }),
            );
        }
        let decision = candidate_decision;
        let raw = candidate_raw;
        let image_point = decision.click_point.clone().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "GUI grounding refinement resolved a target without a click_point".to_string(),
            )
        })?;
        let validation = if grounding_mode == "complex" {
            let (validation_point, validation_bbox, validation_capture_state, validation_image) =
                if let (Some(point), Some(state), Some(image_bytes)) = (
                    validation_point_override.clone(),
                    validation_state_override.clone(),
                    validation_image_bytes_override.as_ref(),
                ) {
                    (
                        point,
                        validation_bbox_override.clone(),
                        state,
                        image_bytes.as_slice(),
                    )
                } else {
                    (
                        translate_original_point_to_model(&prepared_grounding_image, &image_point),
                        decision.bbox.as_ref().map(|bbox| {
                            translate_original_bbox_to_model(&prepared_grounding_image, bbox)
                        }),
                        model_capture_state.clone(),
                        prepared_grounding_image.bytes.as_slice(),
                    )
                };
            let validation_image = render_validation_overlay(
                validation_image,
                &validation_point,
                validation_bbox.as_ref(),
            )?;
            Some({
                let (validation_result, validation_raw, validation_text) =
                    request_model_json::<GroundingValidationResponse>(
                        invocation,
                        build_gui_grounding_validation_prompt(
                            request,
                            &validation_capture_state,
                            &validation_point,
                            validation_bbox.as_ref(),
                            validation_zoom_context,
                        ),
                        ModelInputImage {
                            bytes: &validation_image,
                            mime_type: "image/png",
                        },
                        GUI_GROUNDING_VALIDATION_SYSTEM_PROMPT,
                        gui_grounding_validation_output_schema(),
                        "GUI grounding validation",
                    )
                    .await?;
                (
                    validation_result,
                    validation_raw,
                    validation_text,
                    validation_image.len(),
                )
            })
        } else {
            None
        };
        let rejected = validation
            .as_ref()
            .is_some_and(|(validation_result, _, _, _)| {
                validation_result.status != "approved" || !validation_result.approved
            });
        if let JsonValue::Object(fields) = &mut round_artifact {
            fields.insert(
                "selected_candidate".to_string(),
                predictor_outcome_metadata(
                    &decision,
                    decision.click_point.as_ref(),
                    decision.bbox.as_ref(),
                ),
            );
        }
        if let Some((validation_result, validation_raw, _, validation_image_len)) = &validation
            && let JsonValue::Object(fields) = &mut round_artifact
        {
            fields.insert(
                "validation".to_string(),
                serde_json::json!({
                    "triggered": true,
                    "raw": validation_raw,
                    "outcome": {
                        "status": validation_result.status,
                        "approved": validation_result.approved,
                        "confidence": validation_result.confidence,
                        "reason": validation_result.reason,
                        "failure_kind": validation_result.failure_kind,
                        "retry_hint": validation_result.retry_hint,
                    },
                    "overlay_image": {
                        "mime_type": "image/png",
                        "byte_length": validation_image_len,
                    },
                }),
            );
        } else if let JsonValue::Object(fields) = &mut round_artifact {
            fields.insert(
                "validation".to_string(),
                serde_json::json!({
                    "triggered": false,
                }),
            );
        }
        if rejected && round < max_rounds {
            if let Some(reason) = decision.reason.as_deref() {
                append_unique_retry_note(
                    &mut retry_notes,
                    format!("Round {round} predictor rationale: {reason}"),
                );
            }
            if let Some((validation_result, _, _, _)) = &validation {
                let validation_reason = validation_result
                    .reason
                    .as_deref()
                    .unwrap_or("validator rejected the simulated action");
                append_unique_retry_note(
                    &mut retry_notes,
                    format!("Round {round} validator rejected the candidate: {validation_reason}"),
                );
                if let Some(failure_kind) = validation_result.failure_kind.as_deref() {
                    append_unique_retry_note(
                        &mut retry_notes,
                        format!("Round {round} validator failure kind: {failure_kind}"),
                    );
                    if let Some(guidance) = failure_kind_retry_guidance(failure_kind) {
                        append_unique_retry_note(&mut retry_notes, guidance.to_string());
                    }
                }
                if let Some(retry_hint) = validation_result.retry_hint.as_deref() {
                    append_unique_retry_note(
                        &mut retry_notes,
                        format!("Round {round} retry hint: {retry_hint}"),
                    );
                }
            }
            append_unique_retry_note(
                &mut retry_notes,
                format!(
                    "Round {round} rejected_point_image_pixels: ({}, {})",
                    image_point.x.round(),
                    image_point.y.round()
                ),
            );
            if let Some(bbox) = decision.bbox.as_ref() {
                append_unique_retry_note(
                    &mut retry_notes,
                    format!(
                        "Round {round} rejected_bbox_image_pixels: ({}, {}, {}, {})",
                        bbox.x1.round(),
                        bbox.y1.round(),
                        bbox.x2.round(),
                        bbox.y2.round()
                    ),
                );
            }
            let should_generate_guide = should_generate_retry_guide(validation.as_ref().and_then(
                |(validation_result, _, _, _)| validation_result.failure_kind.as_deref(),
            ));
            guide_image = if should_generate_guide {
                let guide_point =
                    translate_original_point_to_model(&prepared_grounding_image, &image_point);
                let guide_bbox = decision
                    .bbox
                    .as_ref()
                    .map(|bbox| translate_original_bbox_to_model(&prepared_grounding_image, bbox));
                Some(render_guide_overlay(
                    &prepared_grounding_image.bytes,
                    &guide_point,
                    guide_bbox.as_ref(),
                )?)
            } else {
                None
            };
            if let JsonValue::Object(fields) = &mut round_artifact {
                fields.insert(
                    "terminal_state".to_string(),
                    JsonValue::String("validator_rejected_retry".to_string()),
                );
                fields.insert(
                    "retry_notes_after_round".to_string(),
                    JsonValue::Array(retry_notes.iter().cloned().map(JsonValue::String).collect()),
                );
                fields.insert(
                    "guide_image_generated".to_string(),
                    JsonValue::Bool(should_generate_guide),
                );
            }
            round_artifacts.push(round_artifact);
            continue;
        }

        if let JsonValue::Object(fields) = &mut round_artifact {
            fields.insert(
                "terminal_state".to_string(),
                JsonValue::String(if rejected {
                    "validator_rejected_final".to_string()
                } else {
                    "accepted".to_string()
                }),
            );
            fields.insert("selected".to_string(), JsonValue::Bool(true));
            fields.insert(
                "retry_notes_after_round".to_string(),
                JsonValue::Array(retry_notes.iter().cloned().map(JsonValue::String).collect()),
            );
        }
        round_artifacts.push(round_artifact);

        selected_round = round as i64;
        final_decision = Some(decision);
        final_raw = Some(raw);
        final_validation = validation;
        break;
    }

    let decision = final_decision.ok_or_else(|| {
        FunctionCallError::RespondToModel("GUI grounding exhausted retry rounds".to_string())
    })?;
    let mut raw = final_raw.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "GUI grounding exhausted retry rounds without raw output".to_string(),
        )
    })?;
    if let Some((validation_result, _, _, _)) = &final_validation
        && (validation_result.status != "approved" || !validation_result.approved)
    {
        emit_gui_grounding_debug(request, grounding_mode, &round_artifacts);
        return Ok(None);
    }
    let image_point = decision.click_point.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "GUI grounding resolved a target without a click_point".to_string(),
        )
    })?;
    let local_point = image_point_within_capture(capture_state, &image_point).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!(
            "GUI grounding resolved `{}` to image point ({}, {}), which falls outside the screenshot.",
            request.target,
            image_point.x.round(),
            image_point.y.round()
        ))
    })?;
    let local_bounds = decision
        .bbox
        .as_ref()
        .and_then(grounding_bbox_to_rect)
        .and_then(|rect| local_rect_within_state(capture_state, &rect))
        .unwrap_or_else(|| HelperRect {
            x: local_point.x.max(0.0),
            y: local_point.y.max(0.0),
            width: 1.0,
            height: 1.0,
        });
    let display_point = image_point_to_display(capture_state, &local_point);
    let display_bounds = image_rect_to_display(capture_state, &local_bounds);
    let (
        selected_attempt,
        validation_triggered,
        validation_status,
        validation_reason,
        validation_confidence,
        rounds_attempted,
    ) = if let Some((validation_result, validation_raw, _, _)) = final_validation {
        if let JsonValue::Object(fields) = &mut raw {
            fields.insert("validation_raw".to_string(), validation_raw);
        }
        (
            if selected_round > 1 {
                "validated_retry"
            } else {
                "validated"
            },
            true,
            validation_result.status,
            validation_result.reason,
            validation_result.confidence,
            selected_round,
        )
    } else {
        (
            if selected_round > 1 {
                "retry"
            } else {
                "initial"
            },
            false,
            "skipped".to_string(),
            None,
            None,
            selected_round,
        )
    };
    annotate_grounding_round_artifacts(&mut raw, &round_artifacts);
    annotate_grounding_raw(
        &mut raw,
        grounding_mode,
        selected_attempt,
        validation_triggered,
        &validation_status,
        validation_reason.as_deref(),
        validation_confidence,
        rounds_attempted,
    );
    Ok(Some(ResolvedTarget {
        window_title: capture_state.capture.window_title.clone(),
        provider: gui_grounding_provider_name(invocation),
        confidence: decision.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
        reason: decision.reason,
        grounding_mode_requested: grounding_mode.to_string(),
        grounding_mode_effective: grounding_mode.to_string(),
        scope: request.scope.map(ToOwned::to_owned),
        point: display_point,
        bounds: display_bounds,
        local_point: Some(local_point),
        local_bounds: Some(local_bounds),
        raw: Some(raw),
        capture_state: capture_state.clone(),
    }))
}
