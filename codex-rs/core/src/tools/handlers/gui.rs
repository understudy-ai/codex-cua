use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::openai_models::InputModality;
use image::DynamicImage;
use image::ImageFormat;
use image::imageops::FilterType;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use sha1::Digest;
use sha1::Sha1;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use tempfile::tempdir;
use tokio::time::Duration;
use tokio::time::sleep;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

const GUI_UNSUPPORTED_MESSAGE: &str = "Native GUI tools are currently supported on macOS only.";
const GUI_IMAGE_UNSUPPORTED_MESSAGE: &str =
    "Native GUI screenshot tools are not allowed because you do not support image inputs";
const DEFAULT_DRAG_DURATION_MS: i64 = 450;
const DEFAULT_DRAG_STEPS: i64 = 24;
const DEFAULT_HOVER_SETTLE_MS: i64 = 200;
const DEFAULT_CLICK_AND_HOLD_MS: i64 = 650;
const DEFAULT_GUI_WAIT_MS: i64 = 1000;
const DEFAULT_POST_ACTION_SETTLE_MS: i64 = 1200;
const DEFAULT_POST_TYPE_SETTLE_MS: i64 = 500;

#[derive(Default)]
pub struct GuiHandler {
    observe_state: Mutex<HashMap<String, ObserveState>>,
}

#[derive(Clone, Debug)]
struct ObserveState {
    capture_x: f64,
    capture_y: f64,
    width: u32,
    height: u32,
    app_name: Option<String>,
    display_index: i64,
    capture_mode: &'static str,
    window_title: Option<String>,
    window_count: Option<i64>,
    window_capture_strategy: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct WindowSelector {
    title: Option<String>,
    title_contains: Option<String>,
    index: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ObserveArgs {
    app: Option<String>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ClickArgs {
    x: f64,
    y: f64,
    button: Option<String>,
    clicks: Option<i64>,
    hold_ms: Option<i64>,
    settle_ms: Option<i64>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    app: Option<String>,
    post_action_settle_ms: Option<i64>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct WaitArgs {
    duration_ms: Option<i64>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    app: Option<String>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DragArgs {
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
    duration_ms: Option<i64>,
    steps: Option<i64>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    app: Option<String>,
    post_action_settle_ms: Option<i64>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScrollArgs {
    delta_y: Option<i64>,
    delta_x: Option<i64>,
    x: Option<f64>,
    y: Option<f64>,
    unit: Option<String>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    app: Option<String>,
    post_action_settle_ms: Option<i64>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TypeArgs {
    text: Option<String>,
    secret_env_var: Option<String>,
    secret_command_env_var: Option<String>,
    replace: Option<bool>,
    submit: Option<bool>,
    strategy: Option<String>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    app: Option<String>,
    post_action_settle_ms: Option<i64>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct KeyArgs {
    key: String,
    modifiers: Option<Vec<String>>,
    repeat: Option<i64>,
    capture_mode: Option<String>,
    window_title: Option<String>,
    window_selector: Option<WindowSelector>,
    app: Option<String>,
    post_action_settle_ms: Option<i64>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MoveArgs {
    x: f64,
    y: f64,
    app: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct HelperCaptureContext {
    app_name: Option<String>,
    cursor: HelperPoint,
    display: HelperDisplayDescriptor,
    window_id: Option<i64>,
    window_title: Option<String>,
    window_bounds: Option<HelperRect>,
    window_count: Option<i64>,
    window_capture_strategy: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HelperPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HelperDisplayDescriptor {
    index: i64,
    bounds: HelperRect,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HelperRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Debug)]
struct ActionEvidence {
    image_url: Option<String>,
    state: ObserveState,
}

pub struct GuiToolOutput {
    body: Vec<FunctionCallOutputContentItem>,
    code_result: JsonValue,
    success: bool,
}

impl GuiToolOutput {
    fn from_text(text: String) -> Self {
        Self {
            code_result: serde_json::json!({ "message": text }),
            body: vec![FunctionCallOutputContentItem::InputText { text }],
            success: true,
        }
    }
}

impl ToolOutput for GuiToolOutput {
    fn log_preview(&self) -> String {
        self.body
            .iter()
            .find_map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn success_for_logging(&self) -> bool {
        self.success
    }

    fn to_response_item(&self, call_id: &str, _payload: &ToolPayload) -> ResponseInputItem {
        ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output: FunctionCallOutputPayload {
                body: FunctionCallOutputBody::ContentItems(self.body.clone()),
                success: Some(self.success),
            },
        }
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        self.code_result.clone()
    }
}

#[async_trait]
impl ToolHandler for GuiHandler {
    type Output = GuiToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        if !cfg!(target_os = "macos") {
            return Err(FunctionCallError::RespondToModel(
                GUI_UNSUPPORTED_MESSAGE.to_string(),
            ));
        }

        match invocation.tool_name.as_str() {
            "gui_observe" => self.handle_observe(invocation).await,
            "gui_wait" => self.handle_wait(invocation).await,
            "gui_click" => self.handle_click(invocation).await,
            "gui_drag" => self.handle_drag(invocation).await,
            "gui_scroll" => self.handle_scroll(invocation).await,
            "gui_type" => self.handle_type(invocation).await,
            "gui_key" => self.handle_key(invocation).await,
            "gui_move" => self.handle_move(invocation).await,
            name => Err(FunctionCallError::RespondToModel(format!(
                "unsupported gui tool `{name}`"
            ))),
        }
    }
}

impl GuiHandler {
    async fn handle_observe(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        if !supports_image_input(&invocation) {
            return Err(FunctionCallError::RespondToModel(
                GUI_IMAGE_UNSUPPORTED_MESSAGE.to_string(),
            ));
        }

        let args = parse_function_args::<ObserveArgs>(&invocation.payload)?;
        let window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let context = capture_context(args.app.as_deref(), true, window_selection.as_ref())?;
        let capture = resolve_capture_target(
            &context,
            args.capture_mode.as_deref(),
            window_selection.is_some(),
            args.app.as_deref().is_some(),
        )?;
        let image_bytes = capture_region(&capture.bounds, capture.width, capture.height)?;
        let image_url = data_url_png(&image_bytes);
        let state = ObserveState {
            capture_x: capture.bounds.x,
            capture_y: capture.bounds.y,
            width: capture.width,
            height: capture.height,
            app_name: context.app_name.clone(),
            display_index: context.display.index,
            capture_mode: capture.mode,
            window_title: capture.window_title.clone(),
            window_count: capture.window_count,
            window_capture_strategy: capture.window_capture_strategy.clone(),
        };
        self.observe_state
            .lock()
            .expect("gui observe state poisoned")
            .insert(
                invocation.session.conversation_id.to_string(),
                state.clone(),
            );

        let app_label = state
            .app_name
            .as_ref()
            .map(|app| format!(" for app `{app}`"))
            .unwrap_or_default();
        let subject = if state.capture_mode == "window" {
            state
                .window_title
                .as_ref()
                .map(|title| format!("window `{title}`"))
                .unwrap_or_else(|| "window".to_string())
        } else {
            format!("display {}", state.display_index)
        };
        let summary = format!(
            "Captured macOS {subject}{app_label} at origin ({}, {}) with size {}x{}. Coordinates for gui_click/gui_drag/gui_scroll are measured from the top-left of this image.",
            state.capture_x.round(),
            state.capture_y.round(),
            state.width,
            state.height
        );

        let mut body = vec![FunctionCallOutputContentItem::InputText {
            text: summary.clone(),
        }];
        if args.return_image.unwrap_or(true) {
            body.push(FunctionCallOutputContentItem::InputImage {
                image_url: image_url.clone(),
                detail: None,
            });
        }

        Ok(GuiToolOutput {
            body,
            code_result: serde_json::json!({
                "message": summary,
                "image_url": image_url,
                "display_index": state.display_index,
                "capture_mode": state.capture_mode,
                "origin_x": state.capture_x,
                "origin_y": state.capture_y,
                "width": state.width,
                "height": state.height,
                "app": state.app_name,
                "window_title": state.window_title,
                "window_count": state.window_count,
                "window_capture_strategy": state.window_capture_strategy,
            }),
            success: true,
        })
    }

    async fn handle_wait(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<WaitArgs>(&invocation.payload)?;
        let mut window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let mut app = normalize_optional_string(args.app.as_deref());
        let mut capture_mode = normalize_optional_string(args.capture_mode.as_deref());
        let return_image = args.return_image.unwrap_or(true);

        if return_image && !supports_image_input(&invocation) {
            return Err(FunctionCallError::RespondToModel(
                GUI_IMAGE_UNSUPPORTED_MESSAGE.to_string(),
            ));
        }

        if app.is_none() && capture_mode.is_none() && window_selection.is_none() {
            if let Some(previous_state) = self.get_observe_state(&invocation) {
                app = previous_state.app_name.clone();
                capture_mode = Some(previous_state.capture_mode.to_string());
                if previous_state.capture_mode == "window" {
                    window_selection =
                        previous_state
                            .window_title
                            .as_ref()
                            .map(|title| WindowSelector {
                                title: Some(title.clone()),
                                title_contains: None,
                                index: None,
                            });
                }
            }
        }

        let wait_ms = args.duration_ms.unwrap_or(DEFAULT_GUI_WAIT_MS);
        if wait_ms < 0 {
            return Err(FunctionCallError::RespondToModel(
                "gui_wait.duration_ms must be zero or a positive integer".to_string(),
            ));
        }
        sleep(Duration::from_millis(wait_ms as u64)).await;

        let context = capture_context(app.as_deref(), true, window_selection.as_ref())?;
        let capture = resolve_capture_target(
            &context,
            capture_mode.as_deref(),
            window_selection.is_some(),
            app.as_deref().is_some(),
        )?;
        let image_bytes = capture_region(&capture.bounds, capture.width, capture.height)?;
        let image_url = data_url_png(&image_bytes);
        let state = ObserveState {
            capture_x: capture.bounds.x,
            capture_y: capture.bounds.y,
            width: capture.width,
            height: capture.height,
            app_name: context.app_name.clone(),
            display_index: context.display.index,
            capture_mode: capture.mode,
            window_title: capture.window_title.clone(),
            window_count: capture.window_count,
            window_capture_strategy: capture.window_capture_strategy.clone(),
        };
        self.observe_state
            .lock()
            .expect("gui observe state poisoned")
            .insert(
                invocation.session.conversation_id.to_string(),
                state.clone(),
            );

        let app_label = state
            .app_name
            .as_ref()
            .map(|app| format!(" for app `{app}`"))
            .unwrap_or_default();
        let subject = if state.capture_mode == "window" {
            state
                .window_title
                .as_ref()
                .map(|title| format!("window `{title}`"))
                .unwrap_or_else(|| "window".to_string())
        } else {
            format!("display {}", state.display_index)
        };
        let summary = format!(
            "Waited {wait_ms}ms, then refreshed macOS {subject}{app_label} at origin ({}, {}) with size {}x{}. Coordinates for gui_click/gui_drag/gui_scroll are measured from the top-left of this image.",
            state.capture_x.round(),
            state.capture_y.round(),
            state.width,
            state.height
        );

        let mut body = vec![FunctionCallOutputContentItem::InputText {
            text: summary.clone(),
        }];
        if return_image {
            body.push(FunctionCallOutputContentItem::InputImage {
                image_url: image_url.clone(),
                detail: None,
            });
        }

        Ok(GuiToolOutput {
            body,
            code_result: serde_json::json!({
                "message": summary,
                "image_url": image_url,
                "waited_ms": wait_ms,
                "display_index": state.display_index,
                "capture_mode": state.capture_mode,
                "origin_x": state.capture_x,
                "origin_y": state.capture_y,
                "width": state.width,
                "height": state.height,
                "app": state.app_name,
                "window_title": state.window_title,
                "window_count": state.window_count,
                "window_capture_strategy": state.window_capture_strategy,
            }),
            success: true,
        })
    }

    async fn handle_click(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<ClickArgs>(&invocation.payload)?;
        let window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let (global_x, global_y, state) = self.resolve_global_point(
            &invocation,
            args.app.as_deref(),
            args.capture_mode.as_deref(),
            window_selection.as_ref(),
            args.x,
            args.y,
        )?;
        let button = args.button.as_deref().unwrap_or("left");
        let clicks = args.clicks.unwrap_or(1);
        let hold_ms = args.hold_ms.unwrap_or(DEFAULT_CLICK_AND_HOLD_MS).max(1);
        let settle_ms = args.settle_ms.unwrap_or(DEFAULT_HOVER_SETTLE_MS).max(1);
        let event_mode = match (button, clicks, args.hold_ms) {
            ("none", 1, None) => "move_cursor",
            ("left", 1, None) => "click",
            ("left", 1, Some(_)) => "click_and_hold",
            ("left", 2, None) => "double_click",
            ("right", 1, None) => "right_click",
            ("none", _, Some(_)) => {
                return Err(FunctionCallError::RespondToModel(
                    "gui_click cannot combine `button: none` with `hold_ms`".to_string(),
                ));
            }
            ("none", other, None) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click with `button: none` only supports a single hover action, got `{other}`"
                )));
            }
            ("left", 2, Some(_)) => {
                return Err(FunctionCallError::RespondToModel(
                    "gui_click cannot combine `clicks: 2` with `hold_ms`".to_string(),
                ));
            }
            ("left", other, _) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click only supports 1 or 2 left clicks, got `{other}`"
                )));
            }
            ("right", other, _) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click only supports a single right click, got `{other}`"
                )));
            }
            (other, _, _) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click.button only supports `left`, `right`, or `none`, got `{other}`"
                )));
            }
        };

        run_gui_event(
            event_mode,
            args.app.as_deref(),
            &[("CODEX_GUI_X", global_x), ("CODEX_GUI_Y", global_y)],
            &[
                ("CODEX_GUI_HOLD_MS", hold_ms.to_string()),
                ("CODEX_GUI_SETTLE_MS", settle_ms.to_string()),
            ],
        )?;

        let evidence = self
            .capture_post_action_evidence(
                &invocation,
                args.app.as_deref(),
                args.capture_mode.as_deref(),
                window_selection.as_ref(),
                args.post_action_settle_ms,
                args.return_image,
                DEFAULT_POST_ACTION_SETTLE_MS,
            )
            .await?;

        let summary = format!(
            "{} at image coordinate ({}, {}) on macOS {} {} (global {}, {}).{} Use gui_wait or gui_observe to verify the resulting UI state before the next risky action.",
            describe_click_action(button, clicks, args.hold_ms.is_some()),
            args.x.round(),
            args.y.round(),
            state.capture_mode,
            describe_capture_subject(&state),
            global_x.round(),
            global_y.round(),
            if evidence.image_url.is_some() {
                " Attached a refreshed GUI evidence screenshot."
            } else {
                ""
            }
        );
        Ok(self.build_action_output(summary, evidence))
    }

    async fn handle_drag(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<DragArgs>(&invocation.payload)?;
        let window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let (from_global_x, from_global_y, state) = self.resolve_global_point(
            &invocation,
            args.app.as_deref(),
            args.capture_mode.as_deref(),
            window_selection.as_ref(),
            args.from_x,
            args.from_y,
        )?;
        let (to_global_x, to_global_y, _) = self.resolve_global_point(
            &invocation,
            args.app.as_deref(),
            args.capture_mode.as_deref(),
            window_selection.as_ref(),
            args.to_x,
            args.to_y,
        )?;
        let duration_ms = args.duration_ms.unwrap_or(DEFAULT_DRAG_DURATION_MS).max(1);
        let steps = args.steps.unwrap_or(DEFAULT_DRAG_STEPS).max(1);

        run_gui_event(
            "drag",
            args.app.as_deref(),
            &[
                ("CODEX_GUI_FROM_X", from_global_x),
                ("CODEX_GUI_FROM_Y", from_global_y),
                ("CODEX_GUI_TO_X", to_global_x),
                ("CODEX_GUI_TO_Y", to_global_y),
            ],
            &[
                ("CODEX_GUI_DURATION_MS", duration_ms.to_string()),
                ("CODEX_GUI_STEPS", steps.to_string()),
            ],
        )?;

        let evidence = self
            .capture_post_action_evidence(
                &invocation,
                args.app.as_deref(),
                args.capture_mode.as_deref(),
                window_selection.as_ref(),
                args.post_action_settle_ms,
                args.return_image,
                DEFAULT_POST_ACTION_SETTLE_MS,
            )
            .await?;
        let summary = format!(
            "Dragged from ({}, {}) to ({}, {}) on macOS {} {}.{} Use gui_wait or gui_observe to confirm the drop landed where you expected.",
            args.from_x.round(),
            args.from_y.round(),
            args.to_x.round(),
            args.to_y.round(),
            state.capture_mode,
            describe_capture_subject(&state),
            if evidence.image_url.is_some() {
                " Attached a refreshed GUI evidence screenshot."
            } else {
                ""
            }
        );
        Ok(self.build_action_output(summary, evidence))
    }

    async fn handle_scroll(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<ScrollArgs>(&invocation.payload)?;
        let window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let delta_x = args.delta_x.unwrap_or(0);
        let delta_y = args.delta_y.unwrap_or(0);
        if delta_x == 0 && delta_y == 0 {
            return Err(FunctionCallError::RespondToModel(
                "gui_scroll requires at least one of `delta_x` or `delta_y`".to_string(),
            ));
        }

        let mut float_env = Vec::new();
        let mut state_for_summary = None;
        if let (Some(x), Some(y)) = (args.x, args.y) {
            let (global_x, global_y, state) = self.resolve_global_point(
                &invocation,
                args.app.as_deref(),
                args.capture_mode.as_deref(),
                window_selection.as_ref(),
                x,
                y,
            )?;
            float_env.push(("CODEX_GUI_X", global_x));
            float_env.push(("CODEX_GUI_Y", global_y));
            state_for_summary = Some(state);
        } else if args.x.is_some() || args.y.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "gui_scroll requires both `x` and `y` when specifying a scroll location"
                    .to_string(),
            ));
        }

        let unit = args.unit.as_deref().unwrap_or("line");
        if unit != "line" && unit != "pixel" {
            return Err(FunctionCallError::RespondToModel(format!(
                "gui_scroll.unit only supports `line` or `pixel`, got `{unit}`"
            )));
        }

        run_gui_event(
            "scroll",
            args.app.as_deref(),
            &float_env,
            &[
                ("CODEX_GUI_SCROLL_X", delta_x.to_string()),
                ("CODEX_GUI_SCROLL_Y", delta_y.to_string()),
                ("CODEX_GUI_SCROLL_UNIT", unit.to_string()),
            ],
        )?;
        let evidence = self
            .capture_post_action_evidence(
                &invocation,
                args.app.as_deref(),
                args.capture_mode.as_deref(),
                window_selection.as_ref(),
                args.post_action_settle_ms,
                args.return_image,
                DEFAULT_POST_ACTION_SETTLE_MS,
            )
            .await?;
        let summary = format!(
            "Scrolled macOS GUI with delta_x={} delta_y={} ({unit}){}.{} Refresh with gui_wait or gui_observe before grounding the next GUI action.",
            delta_x,
            delta_y,
            state_for_summary
                .as_ref()
                .map(|state| format!(
                    " on {} {}",
                    state.capture_mode,
                    describe_capture_subject(state)
                ))
                .unwrap_or_default(),
            if evidence.image_url.is_some() {
                " Attached a refreshed GUI evidence screenshot."
            } else {
                ""
            }
        );
        Ok(self.build_action_output(summary, evidence))
    }

    async fn handle_type(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<TypeArgs>(&invocation.payload)?;
        let window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let text = resolve_type_text(&args)?;
        let strategy = args.strategy.as_deref().unwrap_or("unicode");
        prepare_targeted_gui_action(
            args.app.as_deref(),
            args.capture_mode.as_deref(),
            window_selection.as_ref(),
        )?;
        if !matches!(
            strategy,
            "unicode"
                | "clipboard_paste"
                | "physical_keys"
                | "system_events_paste"
                | "system_events_keystroke"
                | "system_events_keystroke_chars"
        ) {
            return Err(FunctionCallError::RespondToModel(format!(
                "gui_type.strategy only supports `unicode`, `clipboard_paste`, `physical_keys`, `system_events_paste`, `system_events_keystroke`, or `system_events_keystroke_chars`, got `{strategy}`"
            )));
        }

        if matches!(
            strategy,
            "system_events_paste" | "system_events_keystroke" | "system_events_keystroke_chars"
        ) {
            let replace = args.replace.unwrap_or(true);
            let submit = args.submit.unwrap_or(false);
            run_system_events_type(
                args.app.as_deref(),
                window_selection.as_ref(),
                &text,
                replace,
                submit,
                strategy,
            )?;
        } else {
            let replace = args.replace.unwrap_or(true);
            let submit = args.submit.unwrap_or(false);
            run_gui_event(
                "type_text",
                args.app.as_deref(),
                &[],
                &[
                    ("CODEX_GUI_TEXT", text.clone()),
                    (
                        "CODEX_GUI_REPLACE",
                        if replace { "1" } else { "0" }.to_string(),
                    ),
                    (
                        "CODEX_GUI_SUBMIT",
                        if submit { "1" } else { "0" }.to_string(),
                    ),
                    ("CODEX_GUI_TYPE_STRATEGY", strategy.to_string()),
                ],
            )?;
        }

        let evidence = self
            .capture_post_action_evidence(
                &invocation,
                args.app.as_deref(),
                args.capture_mode.as_deref(),
                window_selection.as_ref(),
                args.post_action_settle_ms,
                args.return_image,
                DEFAULT_POST_TYPE_SETTLE_MS,
            )
            .await?;
        let summary = format!(
            "Typed {} character(s) with strategy `{strategy}`.{} Use gui_wait or gui_observe to verify the field contents and any follow-on UI changes.",
            text.chars().count(),
            if evidence.image_url.is_some() {
                " Attached a refreshed GUI evidence screenshot."
            } else {
                ""
            }
        );
        Ok(self.build_action_output(summary, evidence))
    }

    async fn handle_key(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<KeyArgs>(&invocation.payload)?;
        let window_selection = normalize_window_selection(
            args.window_title.as_deref(),
            args.window_selector.as_ref(),
        )?;
        let repeat = args.repeat.unwrap_or(1).max(1);
        let mut modifiers = args.modifiers.unwrap_or_default();
        let key_code = resolve_key_code(&args.key, &mut modifiers)?;
        let modifiers_env = modifiers.join(",");
        prepare_targeted_gui_action(
            args.app.as_deref(),
            args.capture_mode.as_deref(),
            window_selection.as_ref(),
        )?;

        run_gui_event(
            "key_press",
            args.app.as_deref(),
            &[],
            &[
                ("CODEX_GUI_KEY_CODE", key_code.to_string()),
                ("CODEX_GUI_REPEAT", repeat.to_string()),
                ("CODEX_GUI_MODIFIERS", modifiers_env.clone()),
            ],
        )?;

        let evidence = self
            .capture_post_action_evidence(
                &invocation,
                args.app.as_deref(),
                args.capture_mode.as_deref(),
                window_selection.as_ref(),
                args.post_action_settle_ms,
                args.return_image,
                DEFAULT_POST_TYPE_SETTLE_MS,
            )
            .await?;
        let summary = format!(
            "Pressed key `{}`{} {} time(s).{} Use gui_wait or gui_observe if this shortcut should change the visible UI.",
            args.key,
            if modifiers_env.is_empty() {
                String::new()
            } else {
                format!(" with modifiers [{}]", modifiers_env)
            },
            repeat,
            if evidence.image_url.is_some() {
                " Attached a refreshed GUI evidence screenshot."
            } else {
                ""
            }
        );
        Ok(self.build_action_output(summary, evidence))
    }

    async fn handle_move(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<MoveArgs>(&invocation.payload)?;
        run_gui_event(
            "move_cursor",
            args.app.as_deref(),
            &[("CODEX_GUI_X", args.x), ("CODEX_GUI_Y", args.y)],
            &[("CODEX_GUI_SETTLE_MS", DEFAULT_HOVER_SETTLE_MS.to_string())],
        )?;

        Ok(GuiToolOutput::from_text(format!(
            "Moved the macOS pointer to absolute display coordinate ({}, {}).",
            args.x.round(),
            args.y.round()
        )))
    }

    fn resolve_global_point(
        &self,
        invocation: &ToolInvocation,
        app: Option<&str>,
        capture_mode: Option<&str>,
        window_selection: Option<&WindowSelector>,
        local_x: f64,
        local_y: f64,
    ) -> Result<(f64, f64, ObserveState), FunctionCallError> {
        let session_id = invocation.session.conversation_id.to_string();
        let state = if app.is_none() && capture_mode.is_none() && window_selection.is_none() {
            self.observe_state
                .lock()
                .expect("gui observe state poisoned")
                .get(&session_id)
                .cloned()
        } else {
            None
        };

        let state = match state {
            Some(state) => state,
            None => {
                let context = capture_context(app, false, window_selection)?;
                let capture = resolve_capture_target(
                    &context,
                    capture_mode,
                    window_selection.is_some(),
                    app.is_some(),
                )?;
                ObserveState {
                    capture_x: capture.bounds.x,
                    capture_y: capture.bounds.y,
                    width: capture.width,
                    height: capture.height,
                    app_name: context.app_name,
                    display_index: context.display.index,
                    capture_mode: capture.mode,
                    window_title: capture.window_title,
                    window_count: capture.window_count,
                    window_capture_strategy: capture.window_capture_strategy,
                }
            }
        };

        Ok((state.capture_x + local_x, state.capture_y + local_y, state))
    }

    fn get_observe_state(&self, invocation: &ToolInvocation) -> Option<ObserveState> {
        self.observe_state
            .lock()
            .expect("gui observe state poisoned")
            .get(&invocation.session.conversation_id.to_string())
            .cloned()
    }

    async fn capture_post_action_evidence(
        &self,
        invocation: &ToolInvocation,
        app: Option<&str>,
        capture_mode: Option<&str>,
        window_selection: Option<&WindowSelector>,
        settle_ms: Option<i64>,
        return_image: Option<bool>,
        default_settle_ms: i64,
    ) -> Result<ActionEvidence, FunctionCallError> {
        let attach_image = should_attach_image(invocation, return_image)?;
        let mut app = normalize_optional_string(app);
        let mut capture_mode = normalize_optional_string(capture_mode);
        let mut window_selection = window_selection.cloned();

        if app.is_none() && capture_mode.is_none() && window_selection.is_none() {
            if let Some(previous_state) = self.get_observe_state(invocation) {
                app = previous_state.app_name.clone();
                capture_mode = Some(previous_state.capture_mode.to_string());
                if previous_state.capture_mode == "window" {
                    window_selection =
                        previous_state
                            .window_title
                            .as_ref()
                            .map(|title| WindowSelector {
                                title: Some(title.clone()),
                                title_contains: None,
                                index: None,
                            });
                }
            }
        }

        let settle_ms = settle_ms.unwrap_or(default_settle_ms);
        if settle_ms < 0 {
            return Err(FunctionCallError::RespondToModel(
                "gui post_action_settle_ms must be zero or a positive integer".to_string(),
            ));
        }
        sleep(Duration::from_millis(settle_ms as u64)).await;

        let context = capture_context(app.as_deref(), true, window_selection.as_ref())?;
        let capture = resolve_capture_target(
            &context,
            capture_mode.as_deref(),
            window_selection.is_some(),
            app.as_deref().is_some(),
        )?;
        let image_bytes = if attach_image {
            Some(capture_region(
                &capture.bounds,
                capture.width,
                capture.height,
            )?)
        } else {
            None
        };
        let image_url = image_bytes.as_deref().map(data_url_png);
        let state = ObserveState {
            capture_x: capture.bounds.x,
            capture_y: capture.bounds.y,
            width: capture.width,
            height: capture.height,
            app_name: context.app_name.clone(),
            display_index: context.display.index,
            capture_mode: capture.mode,
            window_title: capture.window_title.clone(),
            window_count: capture.window_count,
            window_capture_strategy: capture.window_capture_strategy.clone(),
        };
        self.observe_state
            .lock()
            .expect("gui observe state poisoned")
            .insert(
                invocation.session.conversation_id.to_string(),
                state.clone(),
            );

        Ok(ActionEvidence { image_url, state })
    }

    fn build_action_output(&self, summary: String, evidence: ActionEvidence) -> GuiToolOutput {
        let mut body = vec![FunctionCallOutputContentItem::InputText {
            text: summary.clone(),
        }];
        if let Some(image_url) = &evidence.image_url {
            body.push(FunctionCallOutputContentItem::InputImage {
                image_url: image_url.clone(),
                detail: None,
            });
        }

        GuiToolOutput {
            body,
            code_result: serde_json::json!({
                "message": summary,
                "image_url": evidence.image_url,
                "display_index": evidence.state.display_index,
                "capture_mode": evidence.state.capture_mode,
                "origin_x": evidence.state.capture_x,
                "origin_y": evidence.state.capture_y,
                "width": evidence.state.width,
                "height": evidence.state.height,
                "app": evidence.state.app_name,
                "window_title": evidence.state.window_title,
                "window_count": evidence.state.window_count,
                "window_capture_strategy": evidence.state.window_capture_strategy,
            }),
            success: true,
        }
    }
}

fn parse_function_args<T>(payload: &ToolPayload) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let ToolPayload::Function { arguments } = payload else {
        return Err(FunctionCallError::RespondToModel(
            "gui handler received unsupported payload".to_string(),
        ));
    };
    parse_arguments(arguments)
}

fn supports_image_input(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .model_info
        .input_modalities
        .contains(&InputModality::Image)
}

fn should_attach_image(
    invocation: &ToolInvocation,
    return_image: Option<bool>,
) -> Result<bool, FunctionCallError> {
    should_attach_image_with_support(supports_image_input(invocation), return_image)
}

fn should_attach_image_with_support(
    image_supported: bool,
    return_image: Option<bool>,
) -> Result<bool, FunctionCallError> {
    match return_image {
        Some(true) if !image_supported => Err(FunctionCallError::RespondToModel(
            GUI_IMAGE_UNSUPPORTED_MESSAGE.to_string(),
        )),
        Some(value) => Ok(value),
        None => Ok(image_supported),
    }
}

fn rounded_dimension(value: f64, label: &str) -> Result<u32, FunctionCallError> {
    let rounded = value.round();
    if !rounded.is_finite() || rounded <= 0.0 || rounded > u32::MAX as f64 {
        return Err(FunctionCallError::RespondToModel(format!(
            "invalid {label} from native GUI runtime: {value}"
        )));
    }
    Ok(rounded as u32)
}

#[derive(Clone, Debug)]
struct CaptureTarget {
    mode: &'static str,
    bounds: HelperRect,
    width: u32,
    height: u32,
    window_title: Option<String>,
    window_count: Option<i64>,
    window_capture_strategy: Option<String>,
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_window_selection(
    window_title: Option<&str>,
    selector: Option<&WindowSelector>,
) -> Result<Option<WindowSelector>, FunctionCallError> {
    let title = normalize_optional_string(window_title).or_else(|| {
        selector
            .and_then(|selector| selector.title.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    });
    let title_contains = selector
        .and_then(|selector| selector.title_contains.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let index = selector.and_then(|selector| selector.index);
    if let Some(index) = index
        && index <= 0
    {
        return Err(FunctionCallError::RespondToModel(
            "gui window_selector.index must be a positive integer".to_string(),
        ));
    }
    if title.is_none() && title_contains.is_none() && index.is_none() {
        return Ok(None);
    }
    Ok(Some(WindowSelector {
        title,
        title_contains,
        index,
    }))
}

fn normalize_capture_mode(
    capture_mode: Option<&str>,
) -> Result<Option<&'static str>, FunctionCallError> {
    match capture_mode
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(None),
        Some("display") => Ok(Some("display")),
        Some("window") => Ok(Some("window")),
        Some(other) => Err(FunctionCallError::RespondToModel(format!(
            "gui capture_mode only supports `display` or `window`, got `{other}`"
        ))),
    }
}

fn resolve_capture_target(
    context: &HelperCaptureContext,
    capture_mode: Option<&str>,
    window_selection_requested: bool,
    prefer_window_when_available: bool,
) -> Result<CaptureTarget, FunctionCallError> {
    let requested_mode = normalize_capture_mode(capture_mode)?;
    if window_selection_requested && context.window_bounds.is_none() {
        return Err(FunctionCallError::RespondToModel(
            "requested macOS window could not be found; check `window_title`/`window_selector` or switch to `capture_mode: \"display\"`"
                .to_string(),
        ));
    }

    let use_window = match requested_mode {
        Some("window") => context.window_bounds.is_some(),
        Some("display") => false,
        None => {
            window_selection_requested
                || (prefer_window_when_available && context.window_bounds.is_some())
        }
        Some(_) => false,
    };

    let (mode, bounds) = if use_window {
        let Some(bounds) = context.window_bounds.clone() else {
            return Err(FunctionCallError::RespondToModel(
                "window capture requested but no matching window bounds were available".to_string(),
            ));
        };
        ("window", bounds)
    } else {
        ("display", context.display.bounds.clone())
    };
    let width = rounded_dimension(bounds.width, "capture width")?;
    let height = rounded_dimension(bounds.height, "capture height")?;

    Ok(CaptureTarget {
        mode,
        bounds,
        width,
        height,
        window_title: if mode == "window" {
            context.window_title.clone()
        } else {
            None
        },
        window_count: if mode == "window" {
            context.window_count
        } else {
            None
        },
        window_capture_strategy: if mode == "window" {
            context.window_capture_strategy.clone()
        } else {
            None
        },
    })
}

fn capture_context(
    app: Option<&str>,
    activate_app: bool,
    window_selection: Option<&WindowSelector>,
) -> Result<HelperCaptureContext, FunctionCallError> {
    let mut env = vec![(
        "CODEX_GUI_ACTIVATE_APP",
        if activate_app {
            "1".to_string()
        } else {
            "0".to_string()
        },
    )];
    if let Some(app) = app.filter(|app| !app.trim().is_empty()) {
        env.push(("CODEX_GUI_APP", app.to_string()));
    }
    if let Some(window_selection) = window_selection {
        if let Some(title) = &window_selection.title {
            env.push(("CODEX_GUI_WINDOW_TITLE", title.clone()));
        }
        if let Some(title_contains) = &window_selection.title_contains {
            env.push(("CODEX_GUI_WINDOW_TITLE_CONTAINS", title_contains.clone()));
        }
        if let Some(index) = window_selection.index {
            env.push(("CODEX_GUI_WINDOW_INDEX", index.to_string()));
        }
    }

    let output = run_helper("capture-context", &env)?;
    serde_json::from_str::<HelperCaptureContext>(&output).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to decode native GUI capture context: {error}"
        ))
    })
}

fn run_gui_event(
    event_mode: &str,
    app: Option<&str>,
    float_env: &[(&str, f64)],
    string_env: &[(&str, String)],
) -> Result<(), FunctionCallError> {
    let mut env = vec![("CODEX_GUI_EVENT_MODE", event_mode.to_string())];
    if let Some(app) = app.filter(|app| !app.trim().is_empty()) {
        env.push(("CODEX_GUI_APP", app.to_string()));
    }
    for (key, value) in float_env {
        env.push(((*key), value.to_string()));
    }
    for (key, value) in string_env {
        env.push(((*key), value.clone()));
    }
    run_helper("event", &env).map(|_| ())
}

fn prepare_targeted_gui_action(
    app: Option<&str>,
    capture_mode: Option<&str>,
    window_selection: Option<&WindowSelector>,
) -> Result<(), FunctionCallError> {
    if app.is_none() && capture_mode.is_none() && window_selection.is_none() {
        return Ok(());
    }

    let context = capture_context(app, true, window_selection)?;
    if capture_mode.is_some() || window_selection.is_some() {
        let _ = resolve_capture_target(
            &context,
            capture_mode,
            window_selection.is_some(),
            app.is_some(),
        )?;
    }
    Ok(())
}

fn describe_capture_subject(state: &ObserveState) -> String {
    if state.capture_mode == "window" {
        state
            .window_title
            .clone()
            .unwrap_or_else(|| "current window".to_string())
    } else {
        format!("display {}", state.display_index)
    }
}

fn describe_click_action(button: &str, clicks: i64, hold: bool) -> String {
    match (button, clicks, hold) {
        ("none", _, _) => "Hovered pointer".to_string(),
        ("left", 1, true) => "Click-and-held".to_string(),
        ("left", 2, _) => "Double-clicked".to_string(),
        ("right", 1, _) => "Right-clicked".to_string(),
        ("left", _, _) => "Clicked".to_string(),
        (other, _, _) => format!("Interacted with button `{other}`"),
    }
}

fn capture_region(
    bounds: &HelperRect,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, FunctionCallError> {
    let dir = tempdir().map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to create temporary directory for GUI screenshot: {error}"
        ))
    })?;
    let image_path = dir.path().join("codex-gui-observe.png");
    let region = format!(
        "{},{},{},{}",
        bounds.x.round(),
        bounds.y.round(),
        bounds.width.round(),
        bounds.height.round()
    );

    let output = Command::new("screencapture")
        .args(["-x", "-C", "-R", &region])
        .arg(&image_path)
        .output()
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!("failed to execute `screencapture`: {error}"))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FunctionCallError::RespondToModel(format!(
            "macOS screenshot capture failed: {}",
            stderr.trim()
        )));
    }

    let bytes = std::fs::read(&image_path).map_err(|error| {
        FunctionCallError::RespondToModel(format!("failed to read captured screenshot: {error}"))
    })?;

    resize_png_to_match_display(bytes, target_width, target_height)
}

fn resize_png_to_match_display(
    bytes: Vec<u8>,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, FunctionCallError> {
    let image = image::load_from_memory(&bytes).map_err(|error| {
        FunctionCallError::RespondToModel(format!("failed to decode captured screenshot: {error}"))
    })?;
    if image.width() == target_width && image.height() == target_height {
        return Ok(bytes);
    }

    let resized = DynamicImage::ImageRgba8(
        image
            .resize_exact(target_width, target_height, FilterType::Triangle)
            .into_rgba8(),
    );
    let mut encoded = Cursor::new(Vec::new());
    resized
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to encode resized GUI screenshot: {error}"
            ))
        })?;
    Ok(encoded.into_inner())
}

fn data_url_png(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", BASE64_STANDARD.encode(bytes))
}

fn run_helper(command: &str, env: &[(&str, String)]) -> Result<String, FunctionCallError> {
    let helper_path = resolve_helper_binary()?;
    let mut cmd = Command::new(helper_path);
    cmd.arg(command);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.output().map_err(|error| {
        FunctionCallError::RespondToModel(format!("failed to execute native GUI helper: {error}"))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FunctionCallError::RespondToModel(format!(
            "native GUI helper failed: {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn resolve_type_text(args: &TypeArgs) -> Result<String, FunctionCallError> {
    let literal_text = args.text.clone();
    let secret_env_var = normalize_optional_string(args.secret_env_var.as_deref());
    let secret_command_env_var = normalize_optional_string(args.secret_command_env_var.as_deref());
    let configured_source_count = [
        literal_text.is_some(),
        secret_env_var.is_some(),
        secret_command_env_var.is_some(),
    ]
    .into_iter()
    .filter(|configured| *configured)
    .count();
    if configured_source_count != 1 {
        return Err(FunctionCallError::RespondToModel(
            "gui_type requires exactly one of `text`, `secret_env_var`, or `secret_command_env_var`"
                .to_string(),
        ));
    }
    if let Some(text) = literal_text {
        return Ok(text);
    }
    if let Some(secret_env_var) = secret_env_var {
        return std::env::var(&secret_env_var).map_err(|_| {
            FunctionCallError::RespondToModel(format!(
                "gui_type secret env var `{secret_env_var}` is missing or empty"
            ))
        });
    }
    let Some(secret_command_env_var) = secret_command_env_var else {
        return Err(FunctionCallError::RespondToModel(
            "gui_type input source could not be resolved".to_string(),
        ));
    };
    let command = std::env::var(&secret_command_env_var).map_err(|_| {
        FunctionCallError::RespondToModel(format!(
            "gui_type secret command env var `{secret_command_env_var}` is missing or empty"
        ))
    })?;
    let output = Command::new("zsh")
        .args(["-lc", &command])
        .output()
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to resolve gui_type secret command `{secret_command_env_var}`: {error}"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FunctionCallError::RespondToModel(format!(
            "gui_type secret command `{secret_command_env_var}` failed: {}",
            stderr.trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(&['\r', '\n'][..])
        .to_string();
    if text.is_empty() {
        return Err(FunctionCallError::RespondToModel(format!(
            "gui_type secret command `{secret_command_env_var}` produced empty output"
        )));
    }
    Ok(text)
}

fn run_apple_script(script: &str, env: &[(&str, String)]) -> Result<String, FunctionCallError> {
    let mut command = Command::new("osascript");
    command.args(["-l", "AppleScript", "-e", script]);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to execute `osascript` for GUI typing: {error}"
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FunctionCallError::RespondToModel(format!(
            "macOS System Events typing failed: {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_system_events_type(
    app: Option<&str>,
    window_selection: Option<&WindowSelector>,
    text: &str,
    replace: bool,
    submit: bool,
    strategy: &str,
) -> Result<(), FunctionCallError> {
    let mut env = Vec::new();
    if let Some(app) = app.filter(|app| !app.trim().is_empty()) {
        env.push(("CODEX_GUI_APP", app.to_string()));
    }
    if let Some(window_selection) = window_selection {
        if let Some(title) = &window_selection.title {
            env.push(("CODEX_GUI_WINDOW_TITLE", title.clone()));
        }
        if let Some(title_contains) = &window_selection.title_contains {
            env.push(("CODEX_GUI_WINDOW_TITLE_CONTAINS", title_contains.clone()));
        }
        if let Some(index) = window_selection.index {
            env.push(("CODEX_GUI_WINDOW_INDEX", index.to_string()));
        }
    }
    env.push(("CODEX_GUI_TEXT", text.to_string()));
    env.push((
        "CODEX_GUI_REPLACE",
        if replace { "1" } else { "0" }.to_string(),
    ));
    env.push((
        "CODEX_GUI_SUBMIT",
        if submit { "1" } else { "0" }.to_string(),
    ));
    env.push((
        "CODEX_GUI_SYSTEM_EVENTS_TYPE_STRATEGY",
        strategy.to_string(),
    ));

    run_apple_script(TYPE_SYSTEM_EVENTS_SCRIPT, &env).map(|_| ())
}

fn resolve_helper_binary() -> Result<PathBuf, FunctionCallError> {
    let mut hasher = Sha1::new();
    hasher.update(HELPER_SOURCE.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let helper_dir = std::env::temp_dir()
        .join("codex-gui-native-helper")
        .join(&hash[..16]);
    let source_path = helper_dir.join("codex-gui-native-helper.swift");
    let binary_path = helper_dir.join("codex-gui-native-helper");

    if binary_path.exists() {
        return Ok(binary_path);
    }

    std::fs::create_dir_all(&helper_dir).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to create native GUI helper directory: {error}"
        ))
    })?;
    std::fs::write(&source_path, HELPER_SOURCE).map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "failed to write native GUI helper source: {error}"
        ))
    })?;

    let output = Command::new("swiftc")
        .arg(&source_path)
        .arg("-o")
        .arg(&binary_path)
        .output()
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to run `swiftc` for native GUI helper: {error}"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FunctionCallError::RespondToModel(format!(
            "failed to compile native GUI helper. Ensure Xcode Command Line Tools are installed and `swiftc` is available. {}",
            stderr.trim()
        )));
    }

    Ok(binary_path)
}

fn resolve_key_code(key: &str, modifiers: &mut Vec<String>) -> Result<i64, FunctionCallError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "gui_key.key must not be empty".to_string(),
        ));
    }

    let normalized = trimmed.to_lowercase();
    let named = match normalized.as_str() {
        "enter" | "return" => Some(36),
        "tab" => Some(48),
        "escape" | "esc" => Some(53),
        "delete" | "backspace" => Some(51),
        "home" => Some(115),
        "pageup" => Some(116),
        "pagedown" => Some(121),
        "end" => Some(119),
        "up" | "arrowup" => Some(126),
        "down" | "arrowdown" => Some(125),
        "left" | "arrowleft" => Some(123),
        "right" | "arrowright" => Some(124),
        "space" | "spacebar" => Some(49),
        _ => None,
    };
    if let Some(code) = named {
        return Ok(code);
    }

    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err(FunctionCallError::RespondToModel(
            "gui_key.key must not be empty".to_string(),
        ));
    };
    if chars.next().is_some() {
        return Err(FunctionCallError::RespondToModel(format!(
            "unsupported gui_key.key `{trimmed}`; use a named key like `Enter` or a single printable character"
        )));
    }

    let (code, needs_shift) = match first {
        'a' | 'A' => (0, first.is_uppercase()),
        's' | 'S' => (1, first.is_uppercase()),
        'd' | 'D' => (2, first.is_uppercase()),
        'f' | 'F' => (3, first.is_uppercase()),
        'h' | 'H' => (4, first.is_uppercase()),
        'g' | 'G' => (5, first.is_uppercase()),
        'z' | 'Z' => (6, first.is_uppercase()),
        'x' | 'X' => (7, first.is_uppercase()),
        'c' | 'C' => (8, first.is_uppercase()),
        'v' | 'V' => (9, first.is_uppercase()),
        'b' | 'B' => (11, first.is_uppercase()),
        'q' | 'Q' => (12, first.is_uppercase()),
        'w' | 'W' => (13, first.is_uppercase()),
        'e' | 'E' => (14, first.is_uppercase()),
        'r' | 'R' => (15, first.is_uppercase()),
        'y' | 'Y' => (16, first.is_uppercase()),
        't' | 'T' => (17, first.is_uppercase()),
        '1' => (18, false),
        '2' => (19, false),
        '3' => (20, false),
        '4' => (21, false),
        '6' => (22, false),
        '5' => (23, false),
        '=' => (24, false),
        '9' => (25, false),
        '7' => (26, false),
        '-' => (27, false),
        '8' => (28, false),
        '0' => (29, false),
        ']' => (30, false),
        'o' | 'O' => (31, first.is_uppercase()),
        'u' | 'U' => (32, first.is_uppercase()),
        '[' => (33, false),
        'i' | 'I' => (34, first.is_uppercase()),
        'p' | 'P' => (35, first.is_uppercase()),
        'l' | 'L' => (37, first.is_uppercase()),
        'j' | 'J' => (38, first.is_uppercase()),
        '\'' => (39, false),
        'k' | 'K' => (40, first.is_uppercase()),
        ';' => (41, false),
        '\\' => (42, false),
        ',' => (43, false),
        '/' => (44, false),
        'n' | 'N' => (45, first.is_uppercase()),
        'm' | 'M' => (46, first.is_uppercase()),
        '.' => (47, false),
        ' ' => (49, false),
        '!' => (18, true),
        '@' => (19, true),
        '#' => (20, true),
        '$' => (21, true),
        '^' => (22, true),
        '%' => (23, true),
        '+' => (24, true),
        '(' => (25, true),
        '&' => (26, true),
        '_' => (27, true),
        '*' => (28, true),
        ')' => (29, true),
        '}' => (30, true),
        '{' => (33, true),
        '"' => (39, true),
        ':' => (41, true),
        '|' => (42, true),
        '<' => (43, true),
        '?' => (44, true),
        '>' => (47, true),
        _ => {
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported gui_key.key `{trimmed}`"
            )));
        }
    };

    if needs_shift
        && !modifiers
            .iter()
            .any(|modifier| modifier.eq_ignore_ascii_case("shift"))
    {
        modifiers.push("shift".to_string());
    }

    Ok(code)
}

static TYPE_SYSTEM_EVENTS_SCRIPT: &str = r##"
on textContains(haystack, needle)
	if needle is "" then return true
	ignoring case
		return (offset of needle in haystack) is not 0
	end ignoring
end textContains

on matchingWindows(targetProc, exactTitle, titleContains)
	set matches to {}
	repeat with candidateWindow in windows of targetProc
		set windowTitle to ""
		try
			set windowTitle to name of candidateWindow as text
		end try
		set exactMatch to true
		if exactTitle is not "" then
			ignoring case
				set exactMatch to windowTitle is exactTitle
			end ignoring
		end if
		set containsMatch to my textContains(windowTitle, titleContains)
		if exactMatch and containsMatch then set end of matches to candidateWindow
	end repeat
	return matches
end matchingWindows

on focusRequestedWindow(targetProc, exactTitle, titleContains, windowIndexText)
	if exactTitle is "" and titleContains is "" and windowIndexText is "" then return
	set matches to my matchingWindows(targetProc, exactTitle, titleContains)
	if (count of matches) is 0 then error "Window not found for the requested selection."
	set targetWindow to item 1 of matches
	if windowIndexText is not "" then
		set requestedIndex to windowIndexText as integer
		if requestedIndex < 1 or requestedIndex > (count of matches) then error "Requested window index is out of range."
		set targetWindow to item requestedIndex of matches
	end if
	try
		tell targetWindow to perform action "AXRaise"
	end try
	try
		tell targetWindow to set value of attribute "AXMain" to true
	end try
	try
		tell targetWindow to set value of attribute "AXFocused" to true
	end try
	delay 0.1
end focusRequestedWindow

on run argv
	set requestedApp to system attribute "CODEX_GUI_APP"
	set requestedWindowTitle to system attribute "CODEX_GUI_WINDOW_TITLE"
	set requestedWindowTitleContains to system attribute "CODEX_GUI_WINDOW_TITLE_CONTAINS"
	set requestedWindowIndex to system attribute "CODEX_GUI_WINDOW_INDEX"
	set replaceText to system attribute "CODEX_GUI_REPLACE"
	set submitText to system attribute "CODEX_GUI_SUBMIT"
	set inputText to system attribute "CODEX_GUI_TEXT"
	set typeStrategy to system attribute "CODEX_GUI_SYSTEM_EVENTS_TYPE_STRATEGY"

	tell application "System Events"
		if requestedApp is not "" then
			if not (exists application process requestedApp) then error "Application process not found: " & requestedApp
			set targetProc to application process requestedApp
			set frontmost of targetProc to true
			delay 0.1
		else
			set targetProc to first application process whose frontmost is true
		end if
		my focusRequestedWindow(targetProc, requestedWindowTitle, requestedWindowTitleContains, requestedWindowIndex)

		if replaceText is "1" then
			keystroke "a" using command down
			delay 0.05
		end if

		if typeStrategy is "system_events_keystroke" then
			keystroke inputText
		else if typeStrategy is "system_events_keystroke_chars" then
			repeat with currentCharacter in characters of inputText
				set typedCharacter to contents of currentCharacter
				if typedCharacter is return or typedCharacter is linefeed then
					key code 36
				else
					keystroke typedCharacter
				end if
				delay 0.055
			end repeat
		else
			set previousClipboard to missing value
			set hadClipboard to false
			try
				set previousClipboard to the clipboard
				set hadClipboard to true
			end try
			set the clipboard to inputText
			delay 0.15
			keystroke "v" using command down
			delay 0.25
			if hadClipboard then
				try
					set the clipboard to previousClipboard
				end try
			end if
		end if

		if submitText is "1" then key code 36
		return "typed"
	end tell
end run
"##;

static HELPER_SOURCE: &str = r##"
import Foundation
import AppKit
import ApplicationServices
import CoreGraphics
import Carbon.HIToolbox

enum HelperError: Error, CustomStringConvertible {
    case invalidCommand(String)
    case invalidEnv(String)
    case missingEnv(String)
    case applicationNotFound(String)
    case activationFailed(String)
    case eventCreationFailed(String)

    var description: String {
        switch self {
        case .invalidCommand(let value):
            return "invalidCommand(\(value))"
        case .invalidEnv(let key):
            return "invalidEnv(\(key))"
        case .missingEnv(let key):
            return "missingEnv(\(key))"
        case .applicationNotFound(let name):
            return "applicationNotFound(\(name))"
        case .activationFailed(let name):
            return "activationFailed(\(name))"
        case .eventCreationFailed(let detail):
            return "eventCreationFailed(\(detail))"
        }
    }
}

struct Rect: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

struct Point: Codable {
    let x: Double
    let y: Double
}

struct DisplayDescriptor: Codable {
    let index: Int
    let bounds: Rect
}

struct CaptureContext: Codable {
    let appName: String?
    let display: DisplayDescriptor
    let cursor: Point
    let windowId: Int?
    let windowTitle: String?
    let windowBounds: Rect?
    let windowCount: Int?
    let windowCaptureStrategy: String?
}

struct WindowMatch {
    let id: Int
    let title: String?
    let bounds: CGRect
    let layer: Int
}

struct WindowSelection {
    let primary: WindowMatch
    let captureBounds: CGRect
    let windowCount: Int
    let captureStrategy: String
}

func env(_ key: String) -> String {
    ProcessInfo.processInfo.environment[key] ?? ""
}

func trimmedEnv(_ key: String) -> String? {
    let value = env(key).trimmingCharacters(in: .whitespacesAndNewlines)
    return value.isEmpty ? nil : value
}

func normalizedText(_ value: String?) -> String? {
    guard let value else { return nil }
    let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    return normalized.isEmpty ? nil : normalized
}

func requiredDouble(_ key: String) throws -> Double {
    let raw = env(key)
    guard let value = Double(raw) else { throw HelperError.invalidEnv(key) }
    return value
}

func requiredInt(_ key: String) throws -> Int {
    let raw = env(key)
    guard let value = Int(raw) else { throw HelperError.invalidEnv(key) }
    return value
}

func requiredInt32(_ key: String) throws -> Int32 {
    let raw = env(key)
    guard let value = Int32(raw) else { throw HelperError.invalidEnv(key) }
    return value
}

func optionalInt(_ key: String) -> Int? {
    let raw = env(key)
    guard !raw.isEmpty else { return nil }
    return Int(raw)
}

func optionalDouble(_ key: String) -> Double? {
    let raw = env(key)
    guard !raw.isEmpty else { return nil }
    return Double(raw)
}

func shouldActivateApp() -> Bool {
    env("CODEX_GUI_ACTIVATE_APP") != "0"
}

func matchesAppName(_ app: NSRunningApplication, requestedName: String) -> Bool {
    let normalized = requestedName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    if normalized.isEmpty { return false }
    if app.localizedName?.lowercased() == normalized { return true }
    if app.bundleIdentifier?.lowercased() == normalized { return true }
    if app.bundleURL?.deletingPathExtension().lastPathComponent.lowercased() == normalized { return true }
    return false
}

func findRunningApplication(named name: String) -> NSRunningApplication? {
    let candidates = NSWorkspace.shared.runningApplications.filter { app in
        matchesAppName(app, requestedName: name) && !app.isTerminated
    }
    if let active = candidates.first(where: { $0.isActive }) {
        return active
    }
    return candidates.first
}

func resolveRequestedApplication(named name: String?) -> NSRunningApplication? {
    guard let name else {
        return NSWorkspace.shared.frontmostApplication
    }
    return findRunningApplication(named: name) ?? NSWorkspace.shared.frontmostApplication
}

func activateApplication(named name: String?) throws {
    guard let name else { return }
    if let frontmost = NSWorkspace.shared.frontmostApplication, matchesAppName(frontmost, requestedName: name) {
        return
    }
    guard let app = findRunningApplication(named: name) else {
        throw HelperError.applicationNotFound(name)
    }
    if !app.activate(options: [.activateIgnoringOtherApps]) {
        throw HelperError.activationFailed(name)
    }
    usleep(100_000)
}

func rect(_ value: CGRect) -> Rect {
    Rect(
        x: value.origin.x.rounded(),
        y: value.origin.y.rounded(),
        width: value.size.width.rounded(),
        height: value.size.height.rounded()
    )
}

func point(_ value: CGPoint) -> Point {
    Point(x: value.x.rounded(), y: value.y.rounded())
}

func activeDisplays() -> [(index: Int, bounds: CGRect)] {
    var count: UInt32 = 0
    CGGetActiveDisplayList(0, nil, &count)
    var displayIDs = Array(repeating: CGDirectDisplayID(0), count: Int(count))
    CGGetActiveDisplayList(count, &displayIDs, &count)
    return Array(displayIDs.prefix(Int(count))).enumerated().map { item in
        (index: item.offset + 1, bounds: CGDisplayBounds(item.element))
    }
}

func displayForPoint(_ point: CGPoint, displays: [(index: Int, bounds: CGRect)]) -> (index: Int, bounds: CGRect) {
    for display in displays where display.bounds.contains(point) {
        return display
    }
    return displays.first ?? (index: 1, bounds: CGDisplayBounds(CGMainDisplayID()))
}

func matchingWindows(
    ownerName: String?,
    exactTitle: String?,
    titleContains: String?
) -> [WindowMatch] {
    guard let windowInfo = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
        return []
    }
    let normalizedExactTitle = normalizedText(exactTitle)
    let normalizedContainsTitle = normalizedText(titleContains)
    var matches: [WindowMatch] = []
    for info in windowInfo {
        let alpha = info[kCGWindowAlpha as String] as? Double ?? 1
        let layer = info[kCGWindowLayer as String] as? Int ?? 0
        let owner = info[kCGWindowOwnerName as String] as? String ?? ""
        if alpha <= 0.01 {
            continue
        }
        if let ownerName, owner != ownerName {
            continue
        }
        guard let rawBounds = info[kCGWindowBounds as String] else {
            continue
        }
        let boundsDict = rawBounds as! CFDictionary
        guard
            let bounds = CGRect(dictionaryRepresentation: boundsDict),
            bounds.width >= 80,
            bounds.height >= 80
        else {
            continue
        }
        let windowId = (info[kCGWindowNumber as String] as? NSNumber)?.intValue
        let title = info[kCGWindowName as String] as? String
        let normalizedTitle = normalizedText(title) ?? ""
        if let normalizedExactTitle, normalizedTitle != normalizedExactTitle {
            continue
        }
        if let normalizedContainsTitle, !normalizedTitle.contains(normalizedContainsTitle) {
            continue
        }
        matches.append(WindowMatch(
            id: windowId ?? 0,
            title: title,
            bounds: bounds.integral,
            layer: layer
        ))
    }
    return matches
}

func rankWindows(_ matches: [WindowMatch]) -> [WindowMatch] {
    return matches.sorted { lhs, rhs in
        let lhsPrimaryLayer = lhs.layer == 0 ? 0 : 1
        let rhsPrimaryLayer = rhs.layer == 0 ? 0 : 1
        if lhsPrimaryLayer != rhsPrimaryLayer {
            return lhsPrimaryLayer < rhsPrimaryLayer
        }
        if lhs.layer != rhs.layer {
            return lhs.layer < rhs.layer
        }
        let lhsArea = lhs.bounds.width * lhs.bounds.height
        let rhsArea = rhs.bounds.width * rhs.bounds.height
        if lhsArea != rhsArea {
            return lhsArea > rhsArea
        }
        let lhsHasTitle = normalizedText(lhs.title) != nil
        let rhsHasTitle = normalizedText(rhs.title) != nil
        if lhsHasTitle != rhsHasTitle {
            return lhsHasTitle && !rhsHasTitle
        }
        return lhs.id < rhs.id
    }
}

func unionBounds(for matches: [WindowMatch]) -> CGRect? {
    guard let first = matches.first else {
        return nil
    }
    return matches.dropFirst().reduce(first.bounds) { partial, match in
        partial.union(match.bounds)
    }.integral
}

func selectedWindow(
    ownerName: String?,
    exactTitle: String?,
    titleContains: String?,
    index: Int?
) -> WindowSelection? {
    let matches = matchingWindows(
        ownerName: ownerName,
        exactTitle: exactTitle,
        titleContains: titleContains
    )
    guard !matches.isEmpty else {
        return nil
    }
    let hasExplicitSelection = normalizedText(exactTitle) != nil || normalizedText(titleContains) != nil || index != nil
    let ranked = rankWindows(matches)
    if let index, index > 0, index <= ranked.count {
        let window = ranked[index - 1]
        return WindowSelection(
            primary: window,
            captureBounds: window.bounds.integral,
            windowCount: 1,
            captureStrategy: "selected_window"
        )
    }
    guard let primary = ranked.first else {
        return nil
    }
    if hasExplicitSelection {
        return WindowSelection(
            primary: primary,
            captureBounds: primary.bounds.integral,
            windowCount: 1,
            captureStrategy: "selected_window"
        )
    }
    if matches.count == 1 {
        return WindowSelection(
            primary: primary,
            captureBounds: primary.bounds.integral,
            windowCount: 1,
            captureStrategy: "main_window"
        )
    }
    guard let combinedBounds = unionBounds(for: matches) else {
        return nil
    }
    return WindowSelection(
        primary: primary,
        captureBounds: combinedBounds,
        windowCount: matches.count,
        captureStrategy: "app_union"
    )
}

func handleCaptureContext() throws {
    let requestedApp = trimmedEnv("CODEX_GUI_APP")
    let requestedWindowTitle = trimmedEnv("CODEX_GUI_WINDOW_TITLE")
    let requestedWindowTitleContains = trimmedEnv("CODEX_GUI_WINDOW_TITLE_CONTAINS")
    let requestedWindowIndex = optionalInt("CODEX_GUI_WINDOW_INDEX")
    if shouldActivateApp() {
        try activateApplication(named: requestedApp)
    }
    let resolvedApp = resolveRequestedApplication(named: requestedApp)
    let targetApp = resolvedApp?.localizedName ?? requestedApp ?? NSWorkspace.shared.frontmostApplication?.localizedName
    let cursorLocation = CGEvent(source: nil)?.location ?? .zero
    let displays = activeDisplays()
    let window = selectedWindow(
        ownerName: targetApp,
        exactTitle: requestedWindowTitle,
        titleContains: requestedWindowTitleContains,
        index: requestedWindowIndex
    )
    let anchorPoint = window.map { CGPoint(x: $0.primary.bounds.midX, y: $0.primary.bounds.midY) } ?? cursorLocation
    let display = displayForPoint(anchorPoint, displays: displays)
    let payload = CaptureContext(
        appName: targetApp,
        display: DisplayDescriptor(index: display.index, bounds: rect(display.bounds)),
        cursor: point(cursorLocation),
        windowId: window?.primary.id,
        windowTitle: window?.primary.title,
        windowBounds: window.map { rect($0.captureBounds) },
        windowCount: window?.windowCount,
        windowCaptureStrategy: window?.captureStrategy
    )
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let data = try encoder.encode(payload)
    FileHandle.standardOutput.write(data)
}

func makeMouseEvent(_ type: CGEventType, point: CGPoint, button: CGMouseButton = .left) throws -> CGEvent {
    guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: button) else {
        throw HelperError.eventCreationFailed("mouse_\(type.rawValue)")
    }
    return event
}

func post(_ event: CGEvent) {
    event.post(tap: .cghidEventTap)
}

func moveCursor(to point: CGPoint) throws {
    post(try makeMouseEvent(.mouseMoved, point: point))
    usleep(30_000)
}

func leftDown(at point: CGPoint, clickState: Int64 = 1) throws {
    let event = try makeMouseEvent(.leftMouseDown, point: point)
    event.setIntegerValueField(.mouseEventClickState, value: clickState)
    post(event)
}

func leftUp(at point: CGPoint, clickState: Int64 = 1) throws {
    let event = try makeMouseEvent(.leftMouseUp, point: point)
    event.setIntegerValueField(.mouseEventClickState, value: clickState)
    post(event)
}

func rightDown(at point: CGPoint, clickState: Int64 = 1) throws {
    let event = try makeMouseEvent(.rightMouseDown, point: point, button: .right)
    event.setIntegerValueField(.mouseEventClickState, value: clickState)
    post(event)
}

func rightUp(at point: CGPoint, clickState: Int64 = 1) throws {
    let event = try makeMouseEvent(.rightMouseUp, point: point, button: .right)
    event.setIntegerValueField(.mouseEventClickState, value: clickState)
    post(event)
}

func drag(from start: CGPoint, to end: CGPoint, steps: Int, durationMs: Int) throws {
    let stepCount = max(1, steps)
    let sleepMicros = useconds_t(max(10_000, (durationMs * 1_000) / stepCount))
    try moveCursor(to: start)
    try leftDown(at: start)
    usleep(50_000)
    for index in 1...stepCount {
        let progress = Double(index) / Double(stepCount)
        let point = CGPoint(
            x: start.x + ((end.x - start.x) * progress),
            y: start.y + ((end.y - start.y) * progress)
        )
        post(try makeMouseEvent(.leftMouseDragged, point: point))
        usleep(sleepMicros)
    }
    try leftUp(at: end)
}

func postKeyboardEvent(keyCode: CGKeyCode, keyDown: Bool, flags: CGEventFlags = []) throws {
    guard let event = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: keyDown) else {
        throw HelperError.eventCreationFailed("keyboard_\(keyCode)_\(keyDown ? "down" : "up")")
    }
    event.flags = flags
    post(event)
}

let shiftKeyCode: CGKeyCode = 56
let controlKeyCode: CGKeyCode = 59
let optionKeyCode: CGKeyCode = 58
let commandKeyCode: CGKeyCode = 55

func modifierKeySequence(for flags: CGEventFlags) -> [(keyCode: CGKeyCode, mask: CGEventFlags)] {
    var sequence: [(keyCode: CGKeyCode, mask: CGEventFlags)] = []
    if flags.contains(.maskControl) { sequence.append((controlKeyCode, .maskControl)) }
    if flags.contains(.maskAlternate) { sequence.append((optionKeyCode, .maskAlternate)) }
    if flags.contains(.maskCommand) { sequence.append((commandKeyCode, .maskCommand)) }
    if flags.contains(.maskShift) { sequence.append((shiftKeyCode, .maskShift)) }
    return sequence
}

func pressKeyCode(_ keyCode: CGKeyCode, flags: CGEventFlags = []) throws {
    let modifierSequence = modifierKeySequence(for: flags)
    var activeFlags: CGEventFlags = []
    for modifier in modifierSequence {
        activeFlags.formUnion(modifier.mask)
        try postKeyboardEvent(keyCode: modifier.keyCode, keyDown: true, flags: activeFlags)
        usleep(20_000)
    }
    try postKeyboardEvent(keyCode: keyCode, keyDown: true, flags: flags)
    usleep(20_000)
    try postKeyboardEvent(keyCode: keyCode, keyDown: false, flags: flags)
    for modifier in modifierSequence.reversed() {
        activeFlags.subtract(modifier.mask)
        try postKeyboardEvent(keyCode: modifier.keyCode, keyDown: false, flags: activeFlags.union(modifier.mask))
        usleep(20_000)
    }
    usleep(20_000)
}

let preferredPhysicalTypingInputSourceIDs = [
    "com.apple.keylayout.ABC",
    "com.apple.keylayout.US"
]

func findInputSource(by id: String) -> TISInputSource? {
    let properties = [kTISPropertyInputSourceID as String: id] as CFDictionary
    guard let listRef = TISCreateInputSourceList(properties, false)?.takeRetainedValue() else {
        return nil
    }
    let sources = listRef as NSArray
    return sources.firstObject as! TISInputSource?
}

func selectPhysicalTypingInputSource() -> TISInputSource? {
    let previous = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue()
    for sourceID in preferredPhysicalTypingInputSourceIDs {
        guard let source = findInputSource(by: sourceID) else { continue }
        if TISSelectInputSource(source) == noErr {
            usleep(250_000)
            break
        }
    }
    return previous
}

func restoreInputSource(_ source: TISInputSource?) {
    guard let source else { return }
    _ = TISSelectInputSource(source)
    usleep(250_000)
}

let baseKeyCodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19,
    "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28,
    "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "l": 37, "j": 38,
    "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47,
    " ": 49
]

let shiftedKeyCodes: [Character: CGKeyCode] = [
    "A": 0, "S": 1, "D": 2, "F": 3, "H": 4, "G": 5, "Z": 6, "X": 7, "C": 8, "V": 9,
    "B": 11, "Q": 12, "W": 13, "E": 14, "R": 15, "Y": 16, "T": 17, "!": 18, "@": 19,
    "#": 20, "$": 21, "^": 22, "%": 23, "+": 24, "(": 25, "&": 26, "_": 27, "*": 28,
    ")": 29, "}": 30, "O": 31, "U": 32, "{": 33, "I": 34, "P": 35, "L": 37, "J": 38,
    "\"": 39, "K": 40, ":": 41, "|": 42, "<": 43, "?": 44, "N": 45, "M": 46, ">": 47
]

func keyPressForCharacter(_ character: Character) throws -> (keyCode: CGKeyCode, flags: CGEventFlags) {
    if let keyCode = baseKeyCodes[character] {
        return (keyCode, [])
    }
    if let keyCode = shiftedKeyCodes[character] {
        return (keyCode, .maskShift)
    }
    throw HelperError.eventCreationFailed("unsupported_physical_key_\(character)")
}

func typeUnicodeText(_ text: String) throws {
    let utf16 = Array(text.utf16)
    guard !utf16.isEmpty else { return }
    guard
        let keyDown = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true),
        let keyUp = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false)
    else {
        throw HelperError.eventCreationFailed("unicode_text")
    }
    keyDown.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: utf16)
    keyUp.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: utf16)
    post(keyDown)
    usleep(30_000)
    post(keyUp)
    usleep(30_000)
}

func typePhysicalKeyText(_ text: String) throws {
    let previousInputSource = selectPhysicalTypingInputSource()
    defer { restoreInputSource(previousInputSource) }
    for character in text {
        let keyPress = try keyPressForCharacter(character)
        try pressKeyCode(keyPress.keyCode, flags: keyPress.flags)
        usleep(90_000)
    }
}

func pasteText(_ text: String) throws {
    let pasteboard = NSPasteboard.general
    let previousString = pasteboard.string(forType: .string)
    pasteboard.clearContents()
    guard pasteboard.setString(text, forType: .string) else {
        throw HelperError.eventCreationFailed("pasteboard_set")
    }
    usleep(100_000)
    try pressKeyCode(9, flags: .maskCommand)
    usleep(150_000)
    pasteboard.clearContents()
    if let previousString {
        _ = pasteboard.setString(previousString, forType: .string)
    }
}

func parseModifierFlags(_ raw: String?) -> CGEventFlags {
    guard let raw else { return [] }
    var flags: CGEventFlags = []
    for modifier in raw.split(separator: ",").map({ $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }) {
        switch modifier {
        case "command":
            flags.insert(.maskCommand)
        case "shift":
            flags.insert(.maskShift)
        case "option", "alt":
            flags.insert(.maskAlternate)
        case "control", "ctrl":
            flags.insert(.maskControl)
        default:
            continue
        }
    }
    return flags
}

func handleEvent() throws {
    let requestedApp = trimmedEnv("CODEX_GUI_APP")
    if shouldActivateApp() {
        try activateApplication(named: requestedApp)
    }

    switch env("CODEX_GUI_EVENT_MODE") {
    case "move_cursor":
        let point = CGPoint(x: try requiredDouble("CODEX_GUI_X"), y: try requiredDouble("CODEX_GUI_Y"))
        let settleMs = max(1, optionalInt("CODEX_GUI_SETTLE_MS") ?? 200)
        try moveCursor(to: point)
        usleep(useconds_t(settleMs * 1_000))
        print("cg_move_cursor")
    case "click":
        let point = CGPoint(x: try requiredDouble("CODEX_GUI_X"), y: try requiredDouble("CODEX_GUI_Y"))
        try moveCursor(to: point)
        try leftDown(at: point)
        usleep(30_000)
        try leftUp(at: point)
        print("cg_click")
    case "click_and_hold":
        let point = CGPoint(x: try requiredDouble("CODEX_GUI_X"), y: try requiredDouble("CODEX_GUI_Y"))
        let holdMs = max(1, optionalInt("CODEX_GUI_HOLD_MS") ?? 650)
        try moveCursor(to: point)
        try leftDown(at: point)
        usleep(useconds_t(holdMs * 1_000))
        try leftUp(at: point)
        print("cg_click_and_hold")
    case "right_click":
        let point = CGPoint(x: try requiredDouble("CODEX_GUI_X"), y: try requiredDouble("CODEX_GUI_Y"))
        try moveCursor(to: point)
        try rightDown(at: point)
        usleep(30_000)
        try rightUp(at: point)
        print("cg_right_click")
    case "double_click":
        let point = CGPoint(x: try requiredDouble("CODEX_GUI_X"), y: try requiredDouble("CODEX_GUI_Y"))
        try moveCursor(to: point)
        for state in [Int64(1), Int64(2)] {
            try leftDown(at: point, clickState: state)
            usleep(30_000)
            try leftUp(at: point, clickState: state)
            usleep(80_000)
        }
        print("cg_double_click")
    case "drag":
        let start = CGPoint(x: try requiredDouble("CODEX_GUI_FROM_X"), y: try requiredDouble("CODEX_GUI_FROM_Y"))
        let end = CGPoint(x: try requiredDouble("CODEX_GUI_TO_X"), y: try requiredDouble("CODEX_GUI_TO_Y"))
        let durationMs = try requiredInt("CODEX_GUI_DURATION_MS")
        let steps = try requiredInt("CODEX_GUI_STEPS")
        try drag(from: start, to: end, steps: steps, durationMs: durationMs)
        print("cg_drag")
    case "scroll":
        if let x = optionalDouble("CODEX_GUI_X"), let y = optionalDouble("CODEX_GUI_Y") {
            try moveCursor(to: CGPoint(x: x, y: y))
        }
        let vertical = try requiredInt32("CODEX_GUI_SCROLL_Y")
        let horizontal = try requiredInt32("CODEX_GUI_SCROLL_X")
        let scrollUnit = trimmedEnv("CODEX_GUI_SCROLL_UNIT")
        let units: CGScrollEventUnit = scrollUnit == "pixel" ? .pixel : .line
        guard let event = CGEvent(
            scrollWheelEvent2Source: nil,
            units: units,
            wheelCount: 2,
            wheel1: vertical,
            wheel2: horizontal,
            wheel3: 0
        ) else {
            throw HelperError.eventCreationFailed("scroll")
        }
        post(event)
        print("cg_scroll")
    case "type_text":
        let rawText = env("CODEX_GUI_TEXT")
        let shouldReplace = env("CODEX_GUI_REPLACE") == "1"
        let shouldSubmit = env("CODEX_GUI_SUBMIT") == "1"
        let typeStrategy = trimmedEnv("CODEX_GUI_TYPE_STRATEGY") ?? "unicode"
        if shouldReplace {
            try pressKeyCode(0, flags: .maskCommand)
        }
        if typeStrategy == "clipboard_paste" {
            try pasteText(rawText)
        } else if typeStrategy == "physical_keys" {
            try typePhysicalKeyText(rawText)
        } else {
            try typeUnicodeText(rawText)
        }
        if shouldSubmit {
            try pressKeyCode(36)
        }
        print("cg_type_text")
    case "key_press":
        let keyCode = CGKeyCode(try requiredInt("CODEX_GUI_KEY_CODE"))
        let repeatCount = max(1, optionalInt("CODEX_GUI_REPEAT") ?? 1)
        let flags = parseModifierFlags(trimmedEnv("CODEX_GUI_MODIFIERS"))
        for _ in 0..<repeatCount {
            try pressKeyCode(keyCode, flags: flags)
            usleep(30_000)
        }
        print("cg_key_press")
    default:
        throw HelperError.missingEnv("CODEX_GUI_EVENT_MODE")
    }
}

do {
    let command = CommandLine.arguments.dropFirst().first ?? ""
    switch command {
    case "capture-context":
        try handleCaptureContext()
    case "event":
        try handleEvent()
    default:
        throw HelperError.invalidCommand(command)
    }
} catch {
    fputs("Codex native GUI helper failed: \(error)\n", stderr)
    exit(1)
}
"##;

#[cfg(test)]
mod tests {
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
        };

        let capture = resolve_capture_target(&context, None, false, true)
            .expect("window should be preferred for in-app work");

        assert_eq!(capture.mode, "window");
        assert_eq!(capture.width, 800);
        assert_eq!(capture.height, 600);
    }

    #[test]
    fn gui_wait_reuses_previous_window_observe_target_when_not_overridden() {
        let previous = ObserveState {
            capture_x: 0.0,
            capture_y: 0.0,
            width: 800,
            height: 600,
            app_name: Some("Notes".to_string()),
            display_index: 1,
            capture_mode: "window",
            window_title: Some("Quick Note".to_string()),
            window_count: Some(1),
            window_capture_strategy: Some("bounds".to_string()),
        };

        let mut app = None;
        let mut capture_mode = None;
        let mut window_selection = None;

        if app.is_none() && capture_mode.is_none() && window_selection.is_none() {
            app = previous.app_name.clone();
            capture_mode = Some(previous.capture_mode.to_string());
            if previous.capture_mode == "window" {
                window_selection = previous.window_title.as_ref().map(|title| WindowSelector {
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
    fn should_attach_image_defaults_to_supported_modalities() {
        assert_eq!(should_attach_image_with_support(true, None).unwrap(), true);
        assert_eq!(
            should_attach_image_with_support(false, None).unwrap(),
            false
        );
        assert_eq!(
            should_attach_image_with_support(true, Some(false)).unwrap(),
            false
        );
        assert!(should_attach_image_with_support(false, Some(true)).is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
    fn macos_gui_capture_smoke_test() {
        let helper_path = resolve_helper_binary().expect("native GUI helper should compile");
        assert!(helper_path.exists(), "helper binary should exist");

        let context =
            capture_context(None, false, None).expect("capture context should be available");
        let capture = resolve_capture_target(&context, Some("display"), false, false)
            .expect("display capture");
        let image_bytes =
            capture_region(&capture.bounds, capture.width, capture.height).expect("screenshot");

        assert!(
            !image_bytes.is_empty(),
            "captured image should not be empty"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "manual macOS GUI smoke test requiring Accessibility permissions"]
    fn macos_gui_move_cursor_smoke_test() {
        let context =
            capture_context(None, false, None).expect("capture context should be available");

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

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
    fn macos_gui_wait_smoke_test() {
        std::thread::sleep(std::time::Duration::from_millis(1));

        let context =
            capture_context(None, false, None).expect("capture context should be available");
        let capture = resolve_capture_target(&context, Some("display"), false, false)
            .expect("display capture");
        let image_bytes =
            capture_region(&capture.bounds, capture.width, capture.height).expect("screenshot");

        assert!(
            !image_bytes.is_empty(),
            "refreshed image should not be empty"
        );
    }
}
