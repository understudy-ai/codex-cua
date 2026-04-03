use serde::Deserialize;
use serde_json::from_str as parse_json;
use sha1::Digest;
use sha1::Sha1;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use tempfile::tempdir;

use crate::function_tool::FunctionCallError;

use super::super::HelperCaptureContext;
use super::super::HelperRect;
use super::super::HostCaptureExclusionState;
use super::super::ObserveState;
use super::super::WindowSelector;
use super::super::readiness::GuiEnvironmentReadinessCheck;
use super::super::readiness::GuiEnvironmentReadinessSnapshot;
use super::super::readiness::GuiReadinessStatus;
use super::GuiEmergencyStopMonitor;
use super::GuiPlatform;
use super::PlatformObservation;

const TYPE_SYSTEM_EVENTS_SCRIPT: &str = include_str!("type_system_events.applescript");
const HELPER_SOURCE: &str = include_str!("native_helper.swift");
const DEFAULT_NATIVE_TYPE_CLEAR_REPEAT: i64 = 48;
const DEFAULT_SYSTEM_EVENTS_PASTE_PRE_DELAY_MS: i64 = 220;
const DEFAULT_SYSTEM_EVENTS_PASTE_POST_DELAY_MS: i64 = 650;
const DEFAULT_SYSTEM_EVENTS_KEYSTROKE_CHAR_DELAY_MS: i64 = 55;

pub(super) struct MacOSPlatform;

#[derive(Clone)]
struct ScriptWindowSelection {
    title: Option<String>,
    title_contains: Option<String>,
    index: Option<i64>,
    bounds: Option<HelperRect>,
}

pub(crate) fn normalize_capture_mode_env(value: &str) -> Option<&'static str> {
    let trimmed = value
        .trim()
        .trim_matches(|char: char| !char.is_ascii_alphabetic());
    if trimmed.eq_ignore_ascii_case("display") {
        Some("display")
    } else if trimmed.eq_ignore_ascii_case("window") {
        Some("window")
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObserveHelperOutput {
    app_name: Option<String>,
    display: super::super::HelperDisplayDescriptor,
    window_title: Option<String>,
    window_count: Option<i64>,
    window_capture_strategy: Option<String>,
    host_self_exclude_applied: Option<bool>,
    host_frontmost_excluded: Option<bool>,
    host_frontmost_app_name: Option<String>,
    host_frontmost_bundle_id: Option<String>,
    capture_mode: String,
    capture_bounds: HelperRect,
    capture_width: u32,
    capture_height: u32,
    image_path: String,
}

const KNOWN_HOST_OWNER_NAME_HINTS: &[(&str, &[&str])] = &[
    ("com.apple.Terminal", &["Terminal"]),
    ("com.googlecode.iterm2", &["iTerm2"]),
    ("com.openai.codex", &["Codex", "Codex Desktop"]),
    ("dev.warp.Warp-Stable", &["Warp"]),
];

fn parse_delimited_values(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(|char: char| char == ',' || char.is_ascii_whitespace())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize_identity(value: &str) -> Option<String> {
    let trimmed = value.trim().to_lowercase();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn dedupe_case_insensitive(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for value in values {
        let Some(identity) = normalize_identity(&value) else {
            continue;
        };
        if seen.insert(identity) {
            deduped.push(value);
        }
    }
    deduped
}

fn title_case_token(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
}

fn derive_owner_name_hints_from_bundle_id(bundle_id: Option<&str>) -> Vec<String> {
    let Some(bundle_id) = bundle_id else {
        return Vec::new();
    };
    let Some(tail) = bundle_id.split('.').next_back() else {
        return Vec::new();
    };
    let normalized = tail.replace(".app", "").replace(['-', '_', '.'], " ");
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Vec::new();
    }
    dedupe_case_insensitive(vec![
        normalized.clone(),
        normalized
            .split(' ')
            .map(title_case_token)
            .collect::<Vec<_>>()
            .join(" "),
    ])
}

fn owner_name_hints_for_bundle_id(bundle_id: Option<&str>) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(bundle_id) = bundle_id {
        for (known_bundle_id, known_hints) in KNOWN_HOST_OWNER_NAME_HINTS {
            if bundle_id.eq_ignore_ascii_case(known_bundle_id) {
                hints.extend(known_hints.iter().map(|hint| (*hint).to_string()));
            }
        }
    }
    hints.extend(derive_owner_name_hints_from_bundle_id(bundle_id));
    hints
}

fn requested_app_targets_host(
    app: Option<&str>,
    bundle_ids: &[String],
    owner_names: &[String],
) -> bool {
    let Some(app) = app.and_then(normalize_identity) else {
        return false;
    };
    bundle_ids
        .iter()
        .filter_map(|candidate| normalize_identity(candidate))
        .any(|candidate| candidate == app)
        || owner_names
            .iter()
            .filter_map(|candidate| normalize_identity(candidate))
            .any(|candidate| candidate == app)
}

fn build_host_window_exclusion_env(app: Option<&str>) -> Vec<(&'static str, String)> {
    let host_bundle_id = std::env::var("__CFBundleIdentifier").ok();
    let configured_bundle_ids =
        parse_delimited_values(std::env::var("CODEX_GUI_EXCLUDED_BUNDLE_IDS").ok());
    let configured_owner_names =
        parse_delimited_values(std::env::var("CODEX_GUI_EXCLUDED_OWNER_NAMES").ok());
    let owner_names = dedupe_case_insensitive(
        configured_owner_names
            .into_iter()
            .chain(owner_name_hints_for_bundle_id(host_bundle_id.as_deref()))
            .chain(parse_delimited_values(
                std::env::var("CODEX_INTERNAL_ORIGINATOR_OVERRIDE").ok(),
            ))
            .chain(parse_delimited_values(std::env::var("TERM_PROGRAM").ok()))
            .collect(),
    );
    let bundle_ids = dedupe_case_insensitive(
        configured_bundle_ids
            .into_iter()
            .chain(host_bundle_id.clone())
            .collect(),
    );
    if (bundle_ids.is_empty() && owner_names.is_empty())
        || requested_app_targets_host(app, &bundle_ids, &owner_names)
    {
        return Vec::new();
    }

    let mut env = Vec::new();
    if !bundle_ids.is_empty() {
        env.push(("CODEX_GUI_AUTO_EXCLUDED_BUNDLE_IDS", bundle_ids.join(",")));
    }
    if !owner_names.is_empty() {
        env.push(("CODEX_GUI_AUTO_EXCLUDED_OWNER_NAMES", owner_names.join(",")));
    }
    env
}

impl GuiPlatform for MacOSPlatform {
    fn readiness_snapshot(&self) -> GuiEnvironmentReadinessSnapshot {
        let mut checks = vec![GuiEnvironmentReadinessCheck {
            id: "platform",
            label: "Platform",
            status: GuiReadinessStatus::Ok,
            summary: "macOS GUI runtime is available on this host.".to_string(),
            detail: None,
        }];

        match run_swift_boolean_check(
            r#"import ApplicationServices
print(AXIsProcessTrusted() ? "1" : "0")"#,
        ) {
            Ok(true) => checks.push(GuiEnvironmentReadinessCheck {
                id: "accessibility",
                label: "Accessibility",
                status: GuiReadinessStatus::Ok,
                summary: "Accessibility permission is granted for native GUI input.".to_string(),
                detail: None,
            }),
            Ok(false) => checks.push(GuiEnvironmentReadinessCheck {
                id: "accessibility",
                label: "Accessibility",
                status: GuiReadinessStatus::Error,
                summary: "Accessibility permission is not granted for native GUI input."
                    .to_string(),
                detail: None,
            }),
            Err(error) => checks.push(GuiEnvironmentReadinessCheck {
                id: "accessibility",
                label: "Accessibility",
                status: GuiReadinessStatus::Warn,
                summary: "Could not confirm Accessibility permission state.".to_string(),
                detail: Some(error.to_string()),
            }),
        }

        match run_swift_boolean_check(
            r#"import CoreGraphics
print(CGPreflightScreenCaptureAccess() ? "1" : "0")"#,
        ) {
            Ok(true) => checks.push(GuiEnvironmentReadinessCheck {
                id: "screen_recording",
                label: "Screen Recording",
                status: GuiReadinessStatus::Ok,
                summary: "Screen Recording permission is granted for GUI screenshots.".to_string(),
                detail: None,
            }),
            Ok(false) => checks.push(GuiEnvironmentReadinessCheck {
                id: "screen_recording",
                label: "Screen Recording",
                status: GuiReadinessStatus::Error,
                summary: "Screen Recording permission is not granted for GUI screenshots."
                    .to_string(),
                detail: None,
            }),
            Err(error) => checks.push(GuiEnvironmentReadinessCheck {
                id: "screen_recording",
                label: "Screen Recording",
                status: GuiReadinessStatus::Warn,
                summary: "Could not confirm Screen Recording permission state.".to_string(),
                detail: Some(error.to_string()),
            }),
        }

        match self.resolve_helper_binary() {
            Ok(path) => checks.push(GuiEnvironmentReadinessCheck {
                id: "native_helper",
                label: "Native GUI Helper",
                status: GuiReadinessStatus::Ok,
                summary: "Native GUI helper is ready for capture and input execution.".to_string(),
                detail: Some(path.display().to_string()),
            }),
            Err(error) => checks.push(GuiEnvironmentReadinessCheck {
                id: "native_helper",
                label: "Native GUI Helper",
                status: GuiReadinessStatus::Error,
                summary: "Native GUI helper is unavailable.".to_string(),
                detail: Some(error.to_string()),
            }),
        }

        let status = if checks
            .iter()
            .all(|check| check.status == GuiReadinessStatus::Unsupported)
        {
            "unsupported"
        } else if checks
            .iter()
            .any(|check| check.status == GuiReadinessStatus::Error)
        {
            "blocked"
        } else if checks
            .iter()
            .any(|check| check.status == GuiReadinessStatus::Warn)
        {
            "degraded"
        } else {
            "ready"
        };
        GuiEnvironmentReadinessSnapshot { status, checks }
    }

    fn resolve_helper_binary(&self) -> Result<PathBuf, FunctionCallError> {
        static CACHED_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        if let Some(path) = CACHED_PATH.get()
            && path.exists()
        {
            return Ok(path.clone());
        }

        let mut hasher = Sha1::new();
        hasher.update(HELPER_SOURCE.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let helper_dir = std::env::temp_dir()
            .join("codex-gui-native-helper")
            .join(&hash[..16]);
        let source_path = helper_dir.join("codex-gui-native-helper.swift");
        let binary_path = helper_dir.join("codex-gui-native-helper");

        if binary_path.exists() {
            let _ = CACHED_PATH.set(binary_path.clone());
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

        // Compile to a temporary path and atomically rename to avoid TOCTOU
        // races when multiple GUI tool calls resolve concurrently.
        let tmp_dir = tempdir().map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to create temp dir for native GUI helper compilation: {error}"
            ))
        })?;
        let tmp_binary = tmp_dir.path().join("codex-gui-native-helper");

        let output = Command::new("swiftc")
            .arg(&source_path)
            .arg("-o")
            .arg(&tmp_binary)
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

        // Atomic rename; if another caller raced us, the last rename wins
        // with a fully-formed binary.
        std::fs::rename(&tmp_binary, &binary_path).map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to install native GUI helper binary: {error}"
            ))
        })?;

        let _ = CACHED_PATH.set(binary_path.clone());
        Ok(binary_path)
    }

    fn cleanup_input_state(&self) -> Result<(), FunctionCallError> {
        run_helper(
            self,
            "cleanup",
            &[
                ("CODEX_GUI_RELEASE_MOUSE", "1".to_string()),
                ("CODEX_GUI_RELEASE_MODIFIERS", "1".to_string()),
            ],
        )
        .map(|_| ())
    }

    fn hide_other_apps(&self, app: Option<&str>) -> Result<Vec<i32>, FunctionCallError> {
        let mut env = vec![];
        if let Some(app) = app.filter(|a| !a.trim().is_empty()) {
            env.push(("CODEX_GUI_APP", app.to_string()));
        }
        let output = run_helper(self, "hide-other-apps", &env)?;
        #[derive(serde::Deserialize)]
        struct HideResult {
            #[serde(rename = "hiddenPids")]
            hidden_pids: Vec<i32>,
        }
        let result: HideResult = parse_json(&output).map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse hide-other-apps result: {e}"
            ))
        })?;
        Ok(result.hidden_pids)
    }

    fn unhide_apps(&self, pids: &[i32]) -> Result<(), FunctionCallError> {
        if pids.is_empty() {
            return Ok(());
        }
        let pids_str = pids
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        run_helper(
            self,
            "unhide-apps",
            &[("CODEX_GUI_HIDDEN_PIDS", pids_str)],
        )
        .map(|_| ())
    }

    fn start_emergency_stop_monitor(
        &self,
    ) -> Result<Option<GuiEmergencyStopMonitor>, FunctionCallError> {
        let helper_path = self.resolve_helper_binary()?;
        let child = Command::new(helper_path)
            .arg("monitor-escape")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                FunctionCallError::RespondToModel(format!(
                    "failed to start native GUI emergency stop monitor: {error}"
                ))
            })?;
        Ok(Some(GuiEmergencyStopMonitor::from_child(child)?))
    }

    fn capture_context(
        &self,
        app: Option<&str>,
        activate_app: bool,
        window_selection: Option<&WindowSelector>,
    ) -> Result<HelperCaptureContext, FunctionCallError> {
        let mut env = vec![(
            "CODEX_GUI_ACTIVATE_APP",
            if activate_app { "1" } else { "0" }.to_string(),
        )];
        if let Some(app) = app.filter(|app| !app.trim().is_empty()) {
            env.push(("CODEX_GUI_APP", app.to_string()));
        }
        env.extend(build_host_window_exclusion_env(app));
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

        let output = run_helper(self, "capture-context", &env)?;
        parse_json::<HelperCaptureContext>(&output).map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to decode native GUI capture context: {error}"
            ))
        })
    }

    fn capture_region(
        &self,
        bounds: &HelperRect,
        _target_width: u32,
        _target_height: u32,
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
                FunctionCallError::RespondToModel(format!(
                    "failed to execute `screencapture`: {error}"
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(FunctionCallError::RespondToModel(format!(
                "macOS screenshot capture failed: {}",
                stderr.trim()
            )));
        }

        let bytes = std::fs::read(&image_path).map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to read captured screenshot: {error}"
            ))
        })?;

        Ok(bytes)
    }

    fn observe(
        &self,
        app: Option<&str>,
        activate_app: bool,
        capture_mode: Option<&str>,
        window_selection: Option<&WindowSelector>,
        prefer_window_when_available: bool,
    ) -> Result<PlatformObservation, FunctionCallError> {
        let capture_mode = capture_mode.and_then(normalize_capture_mode_env);
        let mut env = vec![
            (
                "CODEX_GUI_ACTIVATE_APP",
                if activate_app { "1" } else { "0" }.to_string(),
            ),
            (
                "CODEX_GUI_PREFER_WINDOW",
                if prefer_window_when_available {
                    "1"
                } else {
                    "0"
                }
                .to_string(),
            ),
        ];
        if let Some(app) = app.filter(|app| !app.trim().is_empty()) {
            env.push(("CODEX_GUI_APP", app.to_string()));
        }
        env.extend(build_host_window_exclusion_env(app));
        if let Some(capture_mode) = capture_mode.filter(|mode| !mode.trim().is_empty()) {
            env.push(("CODEX_GUI_CAPTURE_MODE", capture_mode.to_string()));
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

        let output = run_helper(self, "observe", &env)?;
        let observed: ObserveHelperOutput = parse_json(&output).map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to decode native GUI observation payload: {error}"
            ))
        })?;
        let host_self_exclude_adjusted = capture_mode.is_none()
            && window_selection.is_none()
            && !prefer_window_when_available
            && observed.host_self_exclude_applied.unwrap_or(false)
            && observed.host_frontmost_excluded.unwrap_or(false)
            && observed.capture_mode == "window"
            && observed.window_title.is_some();
        let mut redaction_count = 0_i64;
        if observed.capture_mode == "display"
            && observed.host_self_exclude_applied.unwrap_or(false)
            && observed.host_frontmost_excluded.unwrap_or(false)
        {
            redaction_count =
                redact_host_windows(self, &observed.image_path, &observed.capture_bounds, app)?;
        }
        let image_path = PathBuf::from(&observed.image_path);
        let bytes = std::fs::read(&image_path).map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to read native GUI observation image: {error}"
            ))
        })?;
        let _ = std::fs::remove_file(&image_path);
        let image_bytes = bytes;
        let (image_width, image_height) = image_dimensions(&image_bytes)?;
        let state = ObserveState {
            capture: super::super::CaptureArtifact {
                origin_x: observed.capture_bounds.x,
                origin_y: observed.capture_bounds.y,
                width: observed.capture_width,
                height: observed.capture_height,
                image_width,
                image_height,
                display_index: observed.display.index,
                capture_mode: match observed.capture_mode.as_str() {
                    "window" => super::super::CaptureMode::Window,
                    _ => super::super::CaptureMode::Display,
                },
                window_title: observed.window_title.clone(),
                window_count: observed.window_count,
                window_capture_strategy: observed.window_capture_strategy.clone(),
                host_exclusion: HostCaptureExclusionState {
                    applied: observed.host_self_exclude_applied.unwrap_or(false),
                    frontmost_excluded: observed.host_frontmost_excluded.unwrap_or(false),
                    adjusted: host_self_exclude_adjusted,
                    frontmost_app_name: observed.host_frontmost_app_name.clone(),
                    frontmost_bundle_id: observed.host_frontmost_bundle_id.clone(),
                    redaction_count,
                },
            },
            app_name: observed.app_name.clone(),
        };
        Ok(PlatformObservation { state, image_bytes })
    }

    fn run_event(
        &self,
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
        run_helper(self, "event", &env).map(|_| ())
    }

    fn run_system_events_type(
        &self,
        app: Option<&str>,
        window_selection: Option<&WindowSelector>,
        text: &str,
        replace: bool,
        submit: bool,
        strategy: &str,
    ) -> Result<(), FunctionCallError> {
        let resolved_window_selection =
            resolve_script_window_selection(self, app, window_selection)?;
        let mut env = Vec::new();
        if let Some(app) = app.filter(|app| !app.trim().is_empty()) {
            env.push(("CODEX_GUI_APP", app.to_string()));
        }
        if let Some(window_selection) = resolved_window_selection.as_ref() {
            if let Some(title) = &window_selection.title {
                env.push(("CODEX_GUI_WINDOW_TITLE", title.clone()));
            }
            if let Some(title_contains) = &window_selection.title_contains {
                env.push(("CODEX_GUI_WINDOW_TITLE_CONTAINS", title_contains.clone()));
            }
            if let Some(index) = window_selection.index {
                env.push(("CODEX_GUI_WINDOW_INDEX", index.to_string()));
            }
            if let Some(bounds) = &window_selection.bounds {
                env.push(("CODEX_GUI_WINDOW_BOUNDS_X", bounds.x.to_string()));
                env.push(("CODEX_GUI_WINDOW_BOUNDS_Y", bounds.y.to_string()));
                env.push(("CODEX_GUI_WINDOW_BOUNDS_WIDTH", bounds.width.to_string()));
                env.push(("CODEX_GUI_WINDOW_BOUNDS_HEIGHT", bounds.height.to_string()));
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
        let strategy_env = match strategy {
            "system_events_keystroke" => "keystroke",
            "system_events_keystroke_chars" => "keystroke_chars",
            _ => "paste",
        };
        env.push((
            "CODEX_GUI_SYSTEM_EVENTS_TYPE_STRATEGY",
            strategy_env.to_string(),
        ));
        if replace {
            env.push((
                "CODEX_GUI_CLEAR_REPEAT",
                DEFAULT_NATIVE_TYPE_CLEAR_REPEAT.to_string(),
            ));
        }
        env.push((
            "CODEX_GUI_PASTE_PRE_DELAY_MS",
            DEFAULT_SYSTEM_EVENTS_PASTE_PRE_DELAY_MS.to_string(),
        ));
        env.push((
            "CODEX_GUI_PASTE_POST_DELAY_MS",
            DEFAULT_SYSTEM_EVENTS_PASTE_POST_DELAY_MS.to_string(),
        ));
        if strategy == "system_events_keystroke_chars" {
            env.push((
                "CODEX_GUI_KEYSTROKE_CHAR_DELAY_MS",
                DEFAULT_SYSTEM_EVENTS_KEYSTROKE_CHAR_DELAY_MS.to_string(),
            ));
        }

        run_apple_script(TYPE_SYSTEM_EVENTS_SCRIPT, &env).map(|_| ())
    }
}

fn resolve_script_window_selection(
    platform: &MacOSPlatform,
    app: Option<&str>,
    window_selection: Option<&WindowSelector>,
) -> Result<Option<ScriptWindowSelection>, FunctionCallError> {
    let Some(window_selection) = window_selection else {
        return Ok(None);
    };
    let context = platform.capture_context(app, false, Some(window_selection))?;
    if context.window_bounds.is_none() {
        return Err(FunctionCallError::RespondToModel(
            "requested macOS window could not be found for System Events typing".to_string(),
        ));
    }
    Ok(Some(ScriptWindowSelection {
        title: context
            .window_title
            .clone()
            .or_else(|| window_selection.title.clone()),
        title_contains: if context.window_title.is_some() {
            None
        } else {
            window_selection.title_contains.clone()
        },
        index: window_selection.index,
        bounds: context.window_bounds.clone(),
    }))
}

fn run_swift_boolean_check(script: &str) -> Result<bool, FunctionCallError> {
    let output = Command::new("swift")
        .args(["-e", script])
        .output()
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to run Swift GUI readiness check: {error}"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FunctionCallError::RespondToModel(format!(
            "Swift GUI readiness check failed: {}",
            stderr.trim()
        )));
    }
    match String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        other => Err(FunctionCallError::RespondToModel(format!(
            "unexpected Swift GUI readiness check output `{other}`"
        ))),
    }
}

fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), FunctionCallError> {
    let image = image::load_from_memory(bytes).map_err(|error| {
        FunctionCallError::RespondToModel(format!("failed to decode captured screenshot: {error}"))
    })?;
    Ok((image.width(), image.height()))
}

fn run_helper(
    platform: &MacOSPlatform,
    command: &str,
    env: &[(&str, String)],
) -> Result<String, FunctionCallError> {
    let helper_path = platform.resolve_helper_binary()?;
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

fn redact_host_windows(
    platform: &MacOSPlatform,
    image_path: &str,
    capture_bounds: &HelperRect,
    app: Option<&str>,
) -> Result<i64, FunctionCallError> {
    let mut env = vec![
        ("CODEX_GUI_IMAGE_PATH", image_path.to_string()),
        ("CODEX_GUI_CAPTURE_X", capture_bounds.x.to_string()),
        ("CODEX_GUI_CAPTURE_Y", capture_bounds.y.to_string()),
        ("CODEX_GUI_CAPTURE_WIDTH", capture_bounds.width.to_string()),
        (
            "CODEX_GUI_CAPTURE_HEIGHT",
            capture_bounds.height.to_string(),
        ),
    ];
    env.extend(build_host_window_exclusion_env(app));
    let output = run_helper(platform, "redact-host-windows", &env)?;
    let parsed: serde_json::Value = parse_json(&output).unwrap_or_else(|_| serde_json::json!({}));
    Ok(parsed
        .get("redactionCount")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| output.trim().parse::<i64>().ok())
        .unwrap_or(0)
        .max(0))
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
