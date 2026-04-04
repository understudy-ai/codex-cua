use super::*;

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
    #[allow(dead_code)]
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
    strategy: &'static str,
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
}

#[cfg(target_os = "macos")]
const GUI_DIRECT_COORDINATE_SYSTEM_PROMPT: &str = concat!(
    "You are choosing a direct GUI interaction coordinate inside a screenshot. ",
    "Return JSON only, following the provided schema exactly. ",
    "Choose one best click_point on the visible actionable or editable surface that matches the request. ",
    "Do not rely on hidden DOM or implementation details. ",
    "If the target is not confidently visible, return `status` = `not_found`, `found` = false, and null coordinates."
);

#[cfg(target_os = "macos")]
const GUI_DIRECT_COORDINATE_VALIDATION_SYSTEM_PROMPT: &str = concat!(
    "You are validating a direct GUI coordinate prediction inside a screenshot. ",
    "A highlighted marker indicates the proposed click point and target region. ",
    "Return JSON only, following the provided schema exactly. ",
    "Approve the prediction only when the marked point clearly lands on the requested target."
);

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize)]
struct DirectCoordinateValidationResponse {
    status: String,
    approved: bool,
    #[allow(dead_code)]
    confidence: Option<f64>,
    reason: Option<String>,
    failure_kind: Option<String>,
    retry_hint: Option<String>,
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
fn load_live_codex_auth() -> codex_login::CodexAuth {
    let auth_manager =
        codex_login::AuthManager::shared(developer_codex_home(), true, Default::default());
    auth_manager
        .auth_cached()
        .expect("Codex auth is required for the live grounding benchmark")
}

#[cfg(target_os = "macos")]
pub(super) async fn live_grounding_benchmark_session()
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
fn summarize_grounding_bucket_for_strategy(
    measurements: &[GroundingBenchmarkMeasurement],
    strategy: &'static str,
    label: &str,
    filter: impl Fn(&GroundingBenchmarkMeasurement) -> bool,
) {
    summarize_grounding_bucket(
        measurements,
        &format!("{strategy}:{label}"),
        |measurement| measurement.strategy == strategy && filter(measurement),
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

#[cfg(target_os = "macos")]
fn direct_coordinate_validation_output_schema() -> serde_json::Value {
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

#[cfg(target_os = "macos")]
fn build_gui_direct_coordinate_prompt(
    truth: &GroundingBenchmarkTruth,
    capture_state: &ObserveState,
    retry_notes: &[String],
) -> String {
    let mut prompt = build_gui_grounding_prompt(
        benchmark_request_for_truth(truth),
        capture_state,
        grounding_mode_for_truth(truth),
        retry_notes,
        false,
    );
    prompt.push_str(
        "\nDirect-coordinate mode: choose the final tool coordinate yourself instead of delegating to a separate grounding service.",
    );
    prompt.push_str(
        "\nFirst identify the exact visible target, then place click_point inside its actionable or editable surface.",
    );
    prompt.push_str(
        "\nDo not return a coordinate from a visually salient but semantically different panel, row, dialog, toolbar, or card.",
    );
    prompt.push_str(
        "\nFor buttons, tabs, icon controls, and checkboxes, prefer the visible hit target itself over nearby labels, card bodies, or whitespace.",
    );
    prompt.push_str(
        "\nFor fields, place click_point inside the editable text-entry area, not on the label, icon, or surrounding panel.",
    );
    prompt.push_str(
        "\nIf the request is ambiguous and several candidates partially match, prefer the one whose visible scope and local context best fit the request; otherwise return not_found.",
    );
    prompt
}

#[cfg(target_os = "macos")]
fn append_direct_coordinate_retry_notes(
    retry_notes: &mut Vec<String>,
    truth: &GroundingBenchmarkTruth,
    round: usize,
    predictor_reason: Option<&str>,
    validation: Option<&DirectCoordinateValidationResponse>,
    model_point: Option<&HelperPoint>,
    model_bbox: Option<&GroundingBoundingBox>,
) {
    if let Some(reason) = predictor_reason {
        let note = format!("Round {round} predictor rationale: {reason}");
        if !retry_notes.iter().any(|existing| existing == &note) {
            retry_notes.push(note);
        }
    }
    if let Some(validation) = validation {
        if let Some(reason) = validation.reason.as_deref() {
            let note = format!("Round {round} validator rejected the candidate: {reason}");
            if !retry_notes.iter().any(|existing| existing == &note) {
                retry_notes.push(note);
            }
        }
        if let Some(failure_kind) = validation.failure_kind.as_deref() {
            let note = format!("Round {round} validator failure kind: {failure_kind}");
            if !retry_notes.iter().any(|existing| existing == &note) {
                retry_notes.push(note);
            }
            let guidance = match failure_kind {
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
            };
            if let Some(guidance) = guidance
                && !retry_notes.iter().any(|existing| existing == guidance)
            {
                retry_notes.push(guidance.to_string());
            }
        }
        if let Some(retry_hint) = validation.retry_hint.as_deref() {
            let note = format!("Round {round} retry hint: {retry_hint}");
            if !retry_notes.iter().any(|existing| existing == &note) {
                retry_notes.push(note);
            }
        }
    }
    if let Some(model_point) = model_point {
        let note = format!(
            "Round {round} rejected_point_image_pixels: ({}, {})",
            model_point.x.round(),
            model_point.y.round()
        );
        if !retry_notes.iter().any(|existing| existing == &note) {
            retry_notes.push(note);
        }
    }
    if let Some(model_bbox) = model_bbox {
        let note = format!(
            "Round {round} rejected_bbox_image_pixels: ({}, {}, {}, {})",
            model_bbox.x1.round(),
            model_bbox.y1.round(),
            model_bbox.x2.round(),
            model_bbox.y2.round()
        );
        if !retry_notes.iter().any(|existing| existing == &note) {
            retry_notes.push(note);
        }
    }
    for note in build_not_found_retry_notes(benchmark_request_for_truth(truth), round) {
        if !retry_notes.iter().any(|existing| existing == &note) {
            retry_notes.push(note);
        }
    }
}

#[cfg(target_os = "macos")]
fn grounding_mode_for_truth(truth: &GroundingBenchmarkTruth) -> &'static str {
    if truth.difficulty == "complex" {
        "complex"
    } else {
        "single"
    }
}

#[cfg(target_os = "macos")]
fn tool_name_for_truth(truth: &GroundingBenchmarkTruth) -> &'static str {
    if truth.action == "type" {
        "gui_type"
    } else {
        "gui_click"
    }
}

#[cfg(target_os = "macos")]
fn benchmark_request_for_truth<'a>(truth: &'a GroundingBenchmarkTruth) -> GuiTargetRequest<'a> {
    GuiTargetRequest {
        app: Some("Understudy GUI benchmark"),
        capture_mode: Some("window"),
        window_selection: None,
        target: truth.target.as_str(),
        location_hint: truth.location_hint.as_deref(),
        scope: truth.scope.as_deref(),
        grounding_mode: Some(grounding_mode_for_truth(truth)),
        action: if truth.action == "type" {
            "type"
        } else {
            "click"
        },
        related_target: None,
        related_scope: None,
        related_location_hint: None,
        related_point: None,
    }
}

#[cfg(target_os = "macos")]
async fn benchmark_measure_separate_grounding_strategy(
    invocation: &ToolInvocation,
    truth: &GroundingBenchmarkTruth,
    capture_state: &ObserveState,
    screenshot_bytes: &[u8],
) -> GroundingBenchmarkMeasurement {
    let started_at = std::time::Instant::now();
    let outcome = default_gui_grounding_provider()
        .ground(
            invocation,
            benchmark_request_for_truth(truth),
            capture_state,
            screenshot_bytes,
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
                "[codex-grounding-benchmark] strategy=separate_grounding case={} found=true inside={} distance={:.1}px elapsed={}ms target={}",
                truth.id, inside, distance_px, elapsed_ms, truth.target
            );
            GroundingBenchmarkMeasurement {
                strategy: "separate_grounding",
                case_id: truth.id.clone(),
                kind: truth.kind.clone(),
                prompt_clarity: truth.prompt_clarity.clone(),
                difficulty: truth.difficulty.clone(),
                found: true,
                inside,
                distance_px,
                elapsed_ms,
                error: None,
            }
        }
        Ok(None) => {
            println!(
                "[codex-grounding-benchmark] strategy=separate_grounding case={} found=false inside=false elapsed={}ms target={}",
                truth.id, elapsed_ms, truth.target
            );
            GroundingBenchmarkMeasurement {
                strategy: "separate_grounding",
                case_id: truth.id.clone(),
                kind: truth.kind.clone(),
                prompt_clarity: truth.prompt_clarity.clone(),
                difficulty: truth.difficulty.clone(),
                found: false,
                inside: false,
                distance_px: f64::INFINITY,
                elapsed_ms,
                error: None,
            }
        }
        Err(error) => {
            println!(
                "[codex-grounding-benchmark] strategy=separate_grounding case={} error={} elapsed={}ms target={}",
                truth.id, error, elapsed_ms, truth.target
            );
            GroundingBenchmarkMeasurement {
                strategy: "separate_grounding",
                case_id: truth.id.clone(),
                kind: truth.kind.clone(),
                prompt_clarity: truth.prompt_clarity.clone(),
                difficulty: truth.difficulty.clone(),
                found: false,
                inside: false,
                distance_px: f64::INFINITY,
                elapsed_ms,
                error: Some(error.to_string()),
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn benchmark_measure_main_model_coordinate_strategy(
    invocation: &ToolInvocation,
    truth: &GroundingBenchmarkTruth,
    capture_state: &ObserveState,
    screenshot_bytes: &[u8],
) -> GroundingBenchmarkMeasurement {
    let prepared_image = match prepare_grounding_model_image(
        screenshot_bytes,
        GroundingModelImageConfig {
            logical_width: Some(capture_state.capture.width),
            logical_height: Some(capture_state.capture.height),
            scale_x: Some(capture_state.capture.scale_x()),
            scale_y: Some(capture_state.capture.scale_y()),
            allow_logical_normalization: true,
        },
        "direct coordinate benchmark image",
    ) {
        Ok(image) => image,
        Err(error) => {
            return GroundingBenchmarkMeasurement {
                strategy: "main_model_coordinates",
                case_id: truth.id.clone(),
                kind: truth.kind.clone(),
                prompt_clarity: truth.prompt_clarity.clone(),
                difficulty: truth.difficulty.clone(),
                found: false,
                inside: false,
                distance_px: f64::INFINITY,
                elapsed_ms: 0,
                error: Some(error.to_string()),
            };
        }
    };
    let mut model_capture_state = capture_state.clone();
    model_capture_state.capture.image_width = prepared_image.model_width;
    model_capture_state.capture.image_height = prepared_image.model_height;
    let max_rounds = if truth.difficulty == "complex" { 3 } else { 2 };
    let started_at = std::time::Instant::now();
    let mut retry_notes = Vec::new();

    for round in 1..=max_rounds {
        let outcome = request_model_json_from_image::<GroundingModelResponse>(
            invocation,
            build_gui_direct_coordinate_prompt(truth, &model_capture_state, &retry_notes),
            prepared_image.bytes.as_slice(),
            prepared_image.mime_type,
            GUI_DIRECT_COORDINATE_SYSTEM_PROMPT,
            gui_grounding_output_schema(),
            "GUI direct coordinate benchmark",
        )
        .await;

        match outcome {
            Ok((decision, _, _)) => {
                if decision.status == "not_found" || !decision.found {
                    if round < max_rounds {
                        append_direct_coordinate_retry_notes(
                            &mut retry_notes,
                            truth,
                            round,
                            decision.reason.as_deref(),
                            None,
                            None,
                            None,
                        );
                        continue;
                    }
                    let elapsed_ms = started_at.elapsed().as_millis();
                    println!(
                        "[codex-grounding-benchmark] strategy=main_model_coordinates case={} found=false inside=false elapsed={}ms target={}",
                        truth.id, elapsed_ms, truth.target
                    );
                    return GroundingBenchmarkMeasurement {
                        strategy: "main_model_coordinates",
                        case_id: truth.id.clone(),
                        kind: truth.kind.clone(),
                        prompt_clarity: truth.prompt_clarity.clone(),
                        difficulty: truth.difficulty.clone(),
                        found: false,
                        inside: false,
                        distance_px: f64::INFINITY,
                        elapsed_ms,
                        error: None,
                    };
                }
                if decision.status != "resolved" {
                    return GroundingBenchmarkMeasurement {
                        strategy: "main_model_coordinates",
                        case_id: truth.id.clone(),
                        kind: truth.kind.clone(),
                        prompt_clarity: truth.prompt_clarity.clone(),
                        difficulty: truth.difficulty.clone(),
                        found: false,
                        inside: false,
                        distance_px: f64::INFINITY,
                        elapsed_ms: started_at.elapsed().as_millis(),
                        error: Some(format!(
                            "unsupported direct coordinate status `{}`",
                            decision.status
                        )),
                    };
                }
                let coordinate_space = decision
                    .coordinate_space
                    .as_deref()
                    .unwrap_or("image_pixels");
                if coordinate_space != "image_pixels" {
                    return GroundingBenchmarkMeasurement {
                        strategy: "main_model_coordinates",
                        case_id: truth.id.clone(),
                        kind: truth.kind.clone(),
                        prompt_clarity: truth.prompt_clarity.clone(),
                        difficulty: truth.difficulty.clone(),
                        found: false,
                        inside: false,
                        distance_px: f64::INFINITY,
                        elapsed_ms: started_at.elapsed().as_millis(),
                        error: Some(format!("unsupported coordinate space `{coordinate_space}`")),
                    };
                }
                let Some(model_point) = decision.click_point.as_ref() else {
                    return GroundingBenchmarkMeasurement {
                        strategy: "main_model_coordinates",
                        case_id: truth.id.clone(),
                        kind: truth.kind.clone(),
                        prompt_clarity: truth.prompt_clarity.clone(),
                        difficulty: truth.difficulty.clone(),
                        found: false,
                        inside: false,
                        distance_px: f64::INFINITY,
                        elapsed_ms: started_at.elapsed().as_millis(),
                        error: Some("resolved without click_point".to_string()),
                    };
                };

                let validation_image = match render_validation_overlay(
                    prepared_image.bytes.as_slice(),
                    model_point,
                    decision.bbox.as_ref(),
                ) {
                    Ok(image) => image,
                    Err(error) => {
                        return GroundingBenchmarkMeasurement {
                            strategy: "main_model_coordinates",
                            case_id: truth.id.clone(),
                            kind: truth.kind.clone(),
                            prompt_clarity: truth.prompt_clarity.clone(),
                            difficulty: truth.difficulty.clone(),
                            found: false,
                            inside: false,
                            distance_px: f64::INFINITY,
                            elapsed_ms: started_at.elapsed().as_millis(),
                            error: Some(error.to_string()),
                        };
                    }
                };
                let validation =
                    request_model_json_from_image::<DirectCoordinateValidationResponse>(
                        invocation,
                        build_gui_grounding_validation_prompt(
                            benchmark_request_for_truth(truth),
                            &model_capture_state,
                            model_point,
                            decision.bbox.as_ref(),
                            false,
                        ),
                        validation_image.as_slice(),
                        "image/png",
                        GUI_DIRECT_COORDINATE_VALIDATION_SYSTEM_PROMPT,
                        direct_coordinate_validation_output_schema(),
                        "GUI direct coordinate validation",
                    )
                    .await;

                match validation {
                    Ok((validation, _, _))
                        if validation.status == "approved" && validation.approved =>
                    {
                        let point = translate_model_point_to_original(&prepared_image, model_point);
                        let dx = point.x - truth.point.x;
                        let dy = point.y - truth.point.y;
                        let distance_px = (dx * dx + dy * dy).sqrt();
                        let inside = point_inside_benchmark_box(&point, &truth.box_);
                        let elapsed_ms = started_at.elapsed().as_millis();
                        println!(
                            "[codex-grounding-benchmark] strategy=main_model_coordinates case={} found=true inside={} distance={:.1}px elapsed={}ms target={}",
                            truth.id, inside, distance_px, elapsed_ms, truth.target
                        );
                        return GroundingBenchmarkMeasurement {
                            strategy: "main_model_coordinates",
                            case_id: truth.id.clone(),
                            kind: truth.kind.clone(),
                            prompt_clarity: truth.prompt_clarity.clone(),
                            difficulty: truth.difficulty.clone(),
                            found: true,
                            inside,
                            distance_px,
                            elapsed_ms,
                            error: None,
                        };
                    }
                    Ok((validation, _, _)) => {
                        if round < max_rounds {
                            append_direct_coordinate_retry_notes(
                                &mut retry_notes,
                                truth,
                                round,
                                decision.reason.as_deref(),
                                Some(&validation),
                                Some(model_point),
                                decision.bbox.as_ref(),
                            );
                            continue;
                        }
                        let elapsed_ms = started_at.elapsed().as_millis();
                        println!(
                            "[codex-grounding-benchmark] strategy=main_model_coordinates case={} found=false inside=false elapsed={}ms target={}",
                            truth.id, elapsed_ms, truth.target
                        );
                        return GroundingBenchmarkMeasurement {
                            strategy: "main_model_coordinates",
                            case_id: truth.id.clone(),
                            kind: truth.kind.clone(),
                            prompt_clarity: truth.prompt_clarity.clone(),
                            difficulty: truth.difficulty.clone(),
                            found: false,
                            inside: false,
                            distance_px: f64::INFINITY,
                            elapsed_ms,
                            error: None,
                        };
                    }
                    Err(error) => {
                        return GroundingBenchmarkMeasurement {
                            strategy: "main_model_coordinates",
                            case_id: truth.id.clone(),
                            kind: truth.kind.clone(),
                            prompt_clarity: truth.prompt_clarity.clone(),
                            difficulty: truth.difficulty.clone(),
                            found: false,
                            inside: false,
                            distance_px: f64::INFINITY,
                            elapsed_ms: started_at.elapsed().as_millis(),
                            error: Some(error.to_string()),
                        };
                    }
                }
            }
            Err(error) => {
                return GroundingBenchmarkMeasurement {
                    strategy: "main_model_coordinates",
                    case_id: truth.id.clone(),
                    kind: truth.kind.clone(),
                    prompt_clarity: truth.prompt_clarity.clone(),
                    difficulty: truth.difficulty.clone(),
                    found: false,
                    inside: false,
                    distance_px: f64::INFINITY,
                    elapsed_ms: started_at.elapsed().as_millis(),
                    error: Some(error.to_string()),
                };
            }
        }
    }

    GroundingBenchmarkMeasurement {
        strategy: "main_model_coordinates",
        case_id: truth.id.clone(),
        kind: truth.kind.clone(),
        prompt_clarity: truth.prompt_clarity.clone(),
        difficulty: truth.difficulty.clone(),
        found: false,
        inside: false,
        distance_px: f64::INFINITY,
        elapsed_ms: started_at.elapsed().as_millis(),
        error: Some("direct coordinate benchmark exhausted without returning".to_string()),
    }
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

            display_index: 1,
            capture_mode: CaptureMode::Window,
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
        let invocation = gui_invocation(
            session.clone(),
            turn.clone(),
            tool_name_for_truth(truth),
            serde_json::json!({}),
        );
        measurements.push(
            benchmark_measure_separate_grounding_strategy(
                &invocation,
                truth,
                &capture_state,
                &rendered.screenshot_bytes,
            )
            .await,
        );
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

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual real grounding comparison benchmark requiring local Codex auth and swiftc"]
async fn macos_gui_grounding_benchmark_compares_coordinate_strategies() {
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

            display_index: 1,
            capture_mode: CaptureMode::Window,
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
        let invocation = gui_invocation(
            session.clone(),
            turn.clone(),
            tool_name_for_truth(truth),
            serde_json::json!({}),
        );
        measurements.push(
            benchmark_measure_separate_grounding_strategy(
                &invocation,
                truth,
                &capture_state,
                &rendered.screenshot_bytes,
            )
            .await,
        );
        measurements.push(
            benchmark_measure_main_model_coordinate_strategy(
                &invocation,
                truth,
                &capture_state,
                &rendered.screenshot_bytes,
            )
            .await,
        );
    }

    let separate_measurements = measurements
        .iter()
        .filter(|measurement| measurement.strategy == "separate_grounding")
        .collect::<Vec<_>>();
    let direct_measurements = measurements
        .iter()
        .filter(|measurement| measurement.strategy == "main_model_coordinates")
        .collect::<Vec<_>>();

    println!(
        "[codex-grounding-benchmark] screenshot={} cases={} strategies=2",
        rendered.screenshot_path.display(),
        truths.len()
    );
    for strategy in ["separate_grounding", "main_model_coordinates"] {
        summarize_grounding_bucket_for_strategy(
            &measurements,
            strategy,
            "explicit",
            |measurement| measurement.prompt_clarity == "explicit",
        );
        summarize_grounding_bucket_for_strategy(
            &measurements,
            strategy,
            "ambiguous",
            |measurement| measurement.prompt_clarity == "ambiguous",
        );
        summarize_grounding_bucket_for_strategy(
            &measurements,
            strategy,
            "complex",
            |measurement| measurement.difficulty == "complex",
        );
        summarize_grounding_bucket_for_strategy(&measurements, strategy, "type", |measurement| {
            measurement.kind == "text_field"
        });
    }

    let separate_hit_rate = separate_measurements
        .iter()
        .filter(|measurement| measurement.inside)
        .count() as f64
        / separate_measurements.len() as f64;
    let direct_hit_rate = direct_measurements
        .iter()
        .filter(|measurement| measurement.inside)
        .count() as f64
        / direct_measurements.len() as f64;
    let separate_avg_latency_ms = separate_measurements
        .iter()
        .map(|measurement| measurement.elapsed_ms as f64)
        .sum::<f64>()
        / separate_measurements.len() as f64;
    let direct_avg_latency_ms = direct_measurements
        .iter()
        .map(|measurement| measurement.elapsed_ms as f64)
        .sum::<f64>()
        / direct_measurements.len() as f64;

    let separate_errors = separate_measurements
        .iter()
        .filter(|measurement| measurement.error.is_some())
        .count();
    let direct_errors = direct_measurements
        .iter()
        .filter(|measurement| measurement.error.is_some())
        .count();
    let direct_only_hits = truths
        .iter()
        .filter(|truth| {
            let separate = separate_measurements
                .iter()
                .find(|measurement| measurement.case_id == truth.id)
                .expect("separate measurement should exist");
            let direct = direct_measurements
                .iter()
                .find(|measurement| measurement.case_id == truth.id)
                .expect("direct measurement should exist");
            !separate.inside && direct.inside
        })
        .map(|truth| truth.id.clone())
        .collect::<Vec<_>>();
    let separate_only_hits = truths
        .iter()
        .filter(|truth| {
            let separate = separate_measurements
                .iter()
                .find(|measurement| measurement.case_id == truth.id)
                .expect("separate measurement should exist");
            let direct = direct_measurements
                .iter()
                .find(|measurement| measurement.case_id == truth.id)
                .expect("direct measurement should exist");
            separate.inside && !direct.inside
        })
        .map(|truth| truth.id.clone())
        .collect::<Vec<_>>();

    println!(
        "[codex-grounding-benchmark] compare separate_hit_rate={:.2} direct_hit_rate={:.2} hit_rate_delta={:.2} separate_avg={}ms direct_avg={}ms latency_delta={}ms separate_errors={} direct_errors={}",
        separate_hit_rate,
        direct_hit_rate,
        direct_hit_rate - separate_hit_rate,
        separate_avg_latency_ms.round(),
        direct_avg_latency_ms.round(),
        (direct_avg_latency_ms - separate_avg_latency_ms).round(),
        separate_errors,
        direct_errors
    );
    if !direct_only_hits.is_empty() {
        println!(
            "[codex-grounding-benchmark] direct_only_hits={}",
            direct_only_hits.join(", ")
        );
    }
    if !separate_only_hits.is_empty() {
        println!(
            "[codex-grounding-benchmark] separate_only_hits={}",
            separate_only_hits.join(", ")
        );
    }

    assert!(
        !separate_measurements.is_empty()
            && separate_measurements.len() == direct_measurements.len(),
        "expected both strategies to produce one measurement per case"
    );
}
