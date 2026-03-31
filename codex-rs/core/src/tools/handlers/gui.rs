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

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

const GUI_UNSUPPORTED_MESSAGE: &str = "Native GUI tools are currently supported on macOS only.";
const GUI_IMAGE_UNSUPPORTED_MESSAGE: &str =
    "gui_observe is not allowed because you do not support image inputs";
const DEFAULT_DRAG_DURATION_MS: i64 = 450;
const DEFAULT_DRAG_STEPS: i64 = 24;

#[derive(Default)]
pub struct GuiHandler {
    observe_state: Mutex<HashMap<String, ObserveState>>,
}

#[derive(Clone, Debug)]
struct ObserveState {
    origin_x: f64,
    origin_y: f64,
    width: u32,
    height: u32,
    app_name: Option<String>,
    display_index: i64,
}

#[derive(Debug, Deserialize)]
struct ObserveArgs {
    app: Option<String>,
    return_image: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ClickArgs {
    x: f64,
    y: f64,
    button: Option<String>,
    clicks: Option<i64>,
    app: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DragArgs {
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
    duration_ms: Option<i64>,
    steps: Option<i64>,
    app: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScrollArgs {
    delta_y: Option<i64>,
    delta_x: Option<i64>,
    x: Option<f64>,
    y: Option<f64>,
    unit: Option<String>,
    app: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TypeArgs {
    text: String,
    replace: Option<bool>,
    submit: Option<bool>,
    strategy: Option<String>,
    app: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KeyArgs {
    key: String,
    modifiers: Option<Vec<String>>,
    repeat: Option<i64>,
    app: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct HelperCaptureContext {
    app_name: Option<String>,
    cursor: HelperPoint,
    display: HelperDisplayDescriptor,
}

#[derive(Debug, Deserialize, Serialize)]
struct HelperPoint {
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct HelperDisplayDescriptor {
    index: i64,
    bounds: HelperRect,
}

#[derive(Debug, Deserialize, Serialize)]
struct HelperRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
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
            "gui_click" => self.handle_click(invocation).await,
            "gui_drag" => self.handle_drag(invocation).await,
            "gui_scroll" => self.handle_scroll(invocation).await,
            "gui_type" => self.handle_type(invocation).await,
            "gui_key" => self.handle_key(invocation).await,
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
        if !invocation
            .turn
            .model_info
            .input_modalities
            .contains(&InputModality::Image)
        {
            return Err(FunctionCallError::RespondToModel(
                GUI_IMAGE_UNSUPPORTED_MESSAGE.to_string(),
            ));
        }

        let args = parse_function_args::<ObserveArgs>(&invocation.payload)?;
        let context = capture_context(args.app.as_deref(), true)?;
        let width = rounded_dimension(context.display.bounds.width, "display width")?;
        let height = rounded_dimension(context.display.bounds.height, "display height")?;
        let image_bytes = capture_display_region(&context.display.bounds, width, height)?;
        let image_url = data_url_png(&image_bytes);
        let state = ObserveState {
            origin_x: context.display.bounds.x,
            origin_y: context.display.bounds.y,
            width,
            height,
            app_name: context.app_name.clone(),
            display_index: context.display.index,
        };
        self.observe_state
            .lock()
            .expect("gui observe state poisoned")
            .insert(
                invocation.session.conversation_id.to_string(),
                state.clone(),
            );

        let summary = format!(
            "Captured macOS display {}{} at origin ({}, {}) with size {}x{}. Coordinates for gui_click/gui_drag/gui_scroll are measured from the top-left of this image.",
            state.display_index,
            state
                .app_name
                .as_ref()
                .map(|app| format!(" for app `{app}`"))
                .unwrap_or_default(),
            state.origin_x.round(),
            state.origin_y.round(),
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
                "origin_x": state.origin_x,
                "origin_y": state.origin_y,
                "width": state.width,
                "height": state.height,
                "app": state.app_name,
            }),
            success: true,
        })
    }

    async fn handle_click(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<ClickArgs>(&invocation.payload)?;
        let (global_x, global_y, state) =
            self.resolve_global_point(&invocation, args.app.as_deref(), args.x, args.y)?;
        let button = args.button.as_deref().unwrap_or("left");
        let clicks = args.clicks.unwrap_or(1);
        let event_mode = match (button, clicks) {
            ("left", 1) => "click",
            ("left", 2) => "double_click",
            ("right", 1) => "right_click",
            ("left", other) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click only supports 1 or 2 left clicks, got `{other}`"
                )));
            }
            ("right", other) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click only supports a single right click, got `{other}`"
                )));
            }
            (other, _) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "gui_click.button only supports `left` or `right`, got `{other}`"
                )));
            }
        };

        run_gui_event(
            event_mode,
            args.app.as_deref(),
            &[("CODEX_GUI_X", global_x), ("CODEX_GUI_Y", global_y)],
            &[],
        )?;

        Ok(GuiToolOutput::from_text(format!(
            "Clicked {} at image coordinate ({}, {}) on macOS display {} (global {}, {}).",
            button,
            args.x.round(),
            args.y.round(),
            state.display_index,
            global_x.round(),
            global_y.round()
        )))
    }

    async fn handle_drag(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<DragArgs>(&invocation.payload)?;
        let (from_global_x, from_global_y, state) =
            self.resolve_global_point(&invocation, args.app.as_deref(), args.from_x, args.from_y)?;
        let (to_global_x, to_global_y, _) =
            self.resolve_global_point(&invocation, args.app.as_deref(), args.to_x, args.to_y)?;
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

        Ok(GuiToolOutput::from_text(format!(
            "Dragged from ({}, {}) to ({}, {}) on macOS display {}.",
            args.from_x.round(),
            args.from_y.round(),
            args.to_x.round(),
            args.to_y.round(),
            state.display_index
        )))
    }

    async fn handle_scroll(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<ScrollArgs>(&invocation.payload)?;
        let delta_x = args.delta_x.unwrap_or(0);
        let delta_y = args.delta_y.unwrap_or(0);
        if delta_x == 0 && delta_y == 0 {
            return Err(FunctionCallError::RespondToModel(
                "gui_scroll requires at least one of `delta_x` or `delta_y`".to_string(),
            ));
        }

        let mut float_env = Vec::new();
        if let (Some(x), Some(y)) = (args.x, args.y) {
            let (global_x, global_y, _) =
                self.resolve_global_point(&invocation, args.app.as_deref(), x, y)?;
            float_env.push(("CODEX_GUI_X", global_x));
            float_env.push(("CODEX_GUI_Y", global_y));
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

        Ok(GuiToolOutput::from_text(format!(
            "Scrolled macOS GUI with delta_x={} delta_y={} ({unit}).",
            delta_x, delta_y
        )))
    }

    async fn handle_type(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<TypeArgs>(&invocation.payload)?;
        let strategy = args.strategy.as_deref().unwrap_or("unicode");
        if !matches!(strategy, "unicode" | "clipboard_paste" | "physical_keys") {
            return Err(FunctionCallError::RespondToModel(format!(
                "gui_type.strategy only supports `unicode`, `clipboard_paste`, or `physical_keys`, got `{strategy}`"
            )));
        }

        run_gui_event(
            "type_text",
            args.app.as_deref(),
            &[],
            &[
                ("CODEX_GUI_TEXT", args.text.clone()),
                (
                    "CODEX_GUI_REPLACE",
                    if args.replace.unwrap_or(false) {
                        "1"
                    } else {
                        "0"
                    }
                    .to_string(),
                ),
                (
                    "CODEX_GUI_SUBMIT",
                    if args.submit.unwrap_or(false) {
                        "1"
                    } else {
                        "0"
                    }
                    .to_string(),
                ),
                ("CODEX_GUI_TYPE_STRATEGY", strategy.to_string()),
            ],
        )?;

        Ok(GuiToolOutput::from_text(format!(
            "Typed {} character(s) with strategy `{strategy}`.",
            args.text.chars().count()
        )))
    }

    async fn handle_key(
        &self,
        invocation: ToolInvocation,
    ) -> Result<GuiToolOutput, FunctionCallError> {
        let args = parse_function_args::<KeyArgs>(&invocation.payload)?;
        let repeat = args.repeat.unwrap_or(1).max(1);
        let mut modifiers = args.modifiers.unwrap_or_default();
        let key_code = resolve_key_code(&args.key, &mut modifiers)?;
        let modifiers_env = modifiers.join(",");

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

        Ok(GuiToolOutput::from_text(format!(
            "Pressed key `{}`{} {} time(s).",
            args.key,
            if modifiers_env.is_empty() {
                String::new()
            } else {
                format!(" with modifiers [{}]", modifiers_env)
            },
            repeat
        )))
    }

    fn resolve_global_point(
        &self,
        invocation: &ToolInvocation,
        app: Option<&str>,
        local_x: f64,
        local_y: f64,
    ) -> Result<(f64, f64, ObserveState), FunctionCallError> {
        let session_id = invocation.session.conversation_id.to_string();
        let state = if app.is_none() {
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
                let context = capture_context(app, false)?;
                let width = rounded_dimension(context.display.bounds.width, "display width")?;
                let height = rounded_dimension(context.display.bounds.height, "display height")?;
                ObserveState {
                    origin_x: context.display.bounds.x,
                    origin_y: context.display.bounds.y,
                    width,
                    height,
                    app_name: context.app_name,
                    display_index: context.display.index,
                }
            }
        };

        Ok((state.origin_x + local_x, state.origin_y + local_y, state))
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

fn rounded_dimension(value: f64, label: &str) -> Result<u32, FunctionCallError> {
    let rounded = value.round();
    if !rounded.is_finite() || rounded <= 0.0 || rounded > u32::MAX as f64 {
        return Err(FunctionCallError::RespondToModel(format!(
            "invalid {label} from native GUI runtime: {value}"
        )));
    }
    Ok(rounded as u32)
}

fn capture_context(
    app: Option<&str>,
    activate_app: bool,
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

fn capture_display_region(
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
}

func env(_ key: String) -> String {
    ProcessInfo.processInfo.environment[key] ?? ""
}

func trimmedEnv(_ key: String) -> String? {
    let value = env(key).trimmingCharacters(in: .whitespacesAndNewlines)
    return value.isEmpty ? nil : value
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

func handleCaptureContext() throws {
    let requestedApp = trimmedEnv("CODEX_GUI_APP")
    if shouldActivateApp() {
        try activateApplication(named: requestedApp)
    }
    let resolvedApp = resolveRequestedApplication(named: requestedApp)
    let targetApp = resolvedApp?.localizedName ?? requestedApp ?? NSWorkspace.shared.frontmostApplication?.localizedName
    let cursorLocation = CGEvent(source: nil)?.location ?? .zero
    let displays = activeDisplays()
    let anchorPoint = resolvedApp?.isActive == true ? cursorLocation : cursorLocation
    let display = displayForPoint(anchorPoint, displays: displays)
    let payload = CaptureContext(
        appName: targetApp,
        display: DisplayDescriptor(index: display.index, bounds: rect(display.bounds)),
        cursor: point(cursorLocation)
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
    case "click":
        let point = CGPoint(x: try requiredDouble("CODEX_GUI_X"), y: try requiredDouble("CODEX_GUI_Y"))
        try moveCursor(to: point)
        try leftDown(at: point)
        usleep(30_000)
        try leftUp(at: point)
        print("cg_click")
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
