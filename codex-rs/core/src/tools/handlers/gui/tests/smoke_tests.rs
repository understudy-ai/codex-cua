use super::*;

#[tokio::test]
async fn prepare_targeted_gui_action_is_noop_without_targeting() {
    prepare_targeted_gui_action(None, None, None)
        .await
        .expect("no-op targeted action");
}

#[cfg(target_os = "macos")]
fn run_applescript(script: &str) -> String {
    let output = std::process::Command::new("osascript")
        .args(["-l", "AppleScript", "-e", script])
        .output()
        .expect("osascript should launch");
    if !output.status.success() {
        panic!(
            "AppleScript failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(target_os = "macos")]
fn close_textedit_without_saving() {
    let _ = std::process::Command::new("osascript")
        .args([
            "-l",
            "AppleScript",
            "-e",
            r#"
tell application "TextEdit"
	if it is running then
		repeat with docRef in documents
			close docRef saving no
		end repeat
		activate
	end if
end tell
"#,
        ])
        .output();
}

#[cfg(target_os = "macos")]
fn read_textedit_document_text() -> String {
    run_applescript(
        r#"
tell application "TextEdit"
	if (count of documents) is 0 then return ""
	return text of document 1
end tell
"#,
    )
}

#[cfg(target_os = "macos")]
fn launch_textedit() {
    let status = std::process::Command::new("open")
        .args(["-a", "TextEdit"])
        .status()
        .expect("open should launch TextEdit");
    assert!(status.success(), "open -a TextEdit should succeed");
    std::thread::sleep(std::time::Duration::from_millis(400));
    run_applescript(r#"tell application "TextEdit" to activate"#);
}

#[cfg(target_os = "macos")]
fn wait_for_textedit_document_text(expected: &str) {
    let started_at = std::time::Instant::now();
    loop {
        let text = read_textedit_document_text();
        if text.contains(expected) {
            return;
        }
        assert!(
            started_at.elapsed() < std::time::Duration::from_secs(10),
            "timed out waiting for TextEdit content `{expected}`, last content was `{text}`"
        );
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
async fn macos_gui_capture_smoke_test() {
    let helper_path = resolve_helper_binary().expect("native GUI helper should compile");
    assert!(helper_path.exists(), "helper binary should exist");

    let observation = observe_platform(None, false, Some("display"), None, false)
        .await
        .expect("display capture");

    assert!(
        !observation.image_bytes.is_empty(),
        "captured image should not be empty"
    );
    assert_eq!(observation.state.capture.capture_mode, CaptureMode::Display);
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Accessibility permissions"]
async fn macos_gui_move_cursor_smoke_test() {
    let context = capture_context(None, false, None)
        .await
        .expect("capture context should be available");

    run_gui_event(
        "move_cursor",
        None,
        &[
            ("CODEX_GUI_X", context.cursor.x),
            ("CODEX_GUI_Y", context.cursor.y),
        ],
        &[("CODEX_GUI_SETTLE_MS", "1".to_string())],
    )
    .await
    .expect("move cursor event should succeed");
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Accessibility permissions"]
async fn macos_gui_move_tool_handler_smoke_test() {
    let context = capture_context(None, false, None)
        .await
        .expect("capture context should be available");
    let (session, mut turn) = crate::codex::make_session_and_context().await;
    let handler = GuiHandler::default();
    let payload = serde_json::json!({
        "x": context.cursor.x,
        "y": context.cursor.y,
    });

    let output = handler
        .handle(gui_invocation(
            Arc::new(session),
            Arc::new(turn),
            "gui_move",
            payload,
        ))
        .await
        .expect("gui_move should succeed through the tool handler");

    assert!(output.success);
    assert_eq!(output.code_result["action_kind"], "move_cursor");
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
async fn macos_gui_wait_smoke_test() {
    std::thread::sleep(std::time::Duration::from_millis(1));

    let context = capture_context(None, false, None)
        .await
        .expect("capture context should be available");
    let capture =
        resolve_capture_target(&context, Some("display"), false, false).expect("display capture");
    let image_bytes = capture_region(&capture.bounds, capture.width, capture.height)
        .await
        .expect("screenshot");

    assert!(
        !image_bytes.is_empty(),
        "refreshed image should not be empty"
    );
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
async fn macos_gui_textedit_typing_smoke_test() {
    close_textedit_without_saving();
    launch_textedit();

    let handler = GuiHandler::default();
    let (session, turn) = crate::codex::make_session_and_context().await;
    let session = Arc::new(session);
    let turn = Arc::new(turn);

    let outcome = async {
        let new_doc = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("gui_key should create a new TextEdit document");
        assert!(new_doc.success);
        assert_eq!(new_doc.code_result["action_kind"], "key_press");

        let type_first_line = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Codex native smoke",
                }),
            ))
            .await
            .expect("gui_type should type into the new document");
        assert!(type_first_line.success);
        assert_eq!(type_first_line.code_result["action_kind"], "type_text");
        wait_for_textedit_document_text("Codex native smoke");

        let enter = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "Enter",
                }),
            ))
            .await
            .expect("gui_key should press Enter");
        assert!(enter.success);
        assert_eq!(enter.code_result["action_kind"], "key_press");

        let type_second_line = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Second line",
                    "replace": false,
                }),
            ))
            .await
            .expect("gui_type should append the second line");
        assert!(type_second_line.success);
        wait_for_textedit_document_text("Codex native smoke\nSecond line");

        let observation = handler
            .handle(gui_invocation(
                session,
                turn,
                "gui_observe",
                serde_json::json!({
                    "app": "TextEdit",
                }),
            ))
            .await
            .expect("gui_observe should capture a refreshed TextEdit screenshot");
        assert!(observation.success);
        assert_eq!(observation.code_result["capture_mode"], "window");
        assert_eq!(observation.code_result["app"], "TextEdit");
        assert!(observation.code_result["image_url"].is_string());

        let final_text = read_textedit_document_text();
        assert!(final_text.contains("Codex native smoke\nSecond line"));
    }
    .await;

    close_textedit_without_saving();
    outcome
}

/// Smoke test: execute multiple targetless actions in a single gui_batch call.
/// Validates that the batch handler correctly executes key + type steps
/// sequentially, and that the result reflects all actions.
#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
async fn macos_gui_batch_targetless_smoke_test() {
    close_textedit_without_saving();
    launch_textedit();

    let handler = GuiHandler::default();
    let (session, turn) = crate::codex::make_session_and_context().await;
    let session = Arc::new(session);
    let turn = Arc::new(turn);

    let outcome = async {
        // First create a new document
        let new_doc = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("gui_key should create a new TextEdit document");
        assert!(new_doc.success);

        std::thread::sleep(std::time::Duration::from_millis(500));

        // Now batch: type line 1, press Enter, type line 2
        let batch_start = std::time::Instant::now();
        let batch_result = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_batch",
                serde_json::json!({
                    "app": "TextEdit",
                    "steps": [
                        { "action": "type", "value": "Batch line one" },
                        { "action": "key", "key": "Return" },
                        { "action": "type", "value": "Batch line two", "replace": false },
                    ]
                }),
            ))
            .await
            .expect("gui_batch should execute all steps");
        let batch_elapsed = batch_start.elapsed();
        assert!(batch_result.success);
        assert_eq!(batch_result.code_result["action_kind"], "batch");
        assert_eq!(batch_result.code_result["steps_count"], 3);
        eprintln!(
            "[gui_batch timing] 3 targetless steps in batch: {:?}",
            batch_elapsed
        );

        wait_for_textedit_document_text("Batch line one\nBatch line two");
        let final_text = read_textedit_document_text();
        assert!(
            final_text.contains("Batch line one\nBatch line two"),
            "TextEdit should contain both lines, got: {final_text}"
        );
    }
    .await;

    close_textedit_without_saving();
    outcome
}

/// 10-step batch with grounded click/type targets on a real app.
/// Uses live Codex credentials to call the grounding model.
/// Measures individual grounded calls vs batch grounding.
#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording, Accessibility, and live Codex auth"]
async fn macos_gui_batch_10_step_grounded_vs_individual() {
    use super::benchmark::live_grounding_benchmark_session;

    close_textedit_without_saving();
    launch_textedit();

    let handler = GuiHandler::default();
    let (session, turn) = live_grounding_benchmark_session().await;

    // Ensure TextEdit has a fresh document with the formatting bar visible.
    let outcome = async {
        // Create new document via Cmd+N.
        let new_doc = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("Cmd+N to create new document");
        assert!(new_doc.success);
        std::thread::sleep(std::time::Duration::from_millis(800));

        // ── A. 10 individual grounded calls ──────────────────────────
        // Each call: screenshot → grounding → action → evidence.
        let targets: Vec<serde_json::Value> = vec![
            serde_json::json!({
                "action": "click",
                "target": "Bold button",
                "scope": "formatting toolbar",
            }),
            serde_json::json!({
                "action": "click",
                "target": "Italic button",
                "scope": "formatting toolbar",
            }),
            serde_json::json!({
                "action": "click",
                "target": "Underline button",
                "scope": "formatting toolbar",
            }),
            serde_json::json!({
                "action": "type",
                "target": "document text area",
                "value": "Hello World",
            }),
            serde_json::json!({
                "action": "key",
                "key": "Return",
            }),
            serde_json::json!({
                "action": "type",
                "value": "Line two",
                "replace": false,
            }),
            serde_json::json!({
                "action": "key",
                "key": "Return",
            }),
            serde_json::json!({
                "action": "type",
                "value": "Line three",
                "replace": false,
            }),
            serde_json::json!({
                "action": "click",
                "target": "Bold button",
                "scope": "formatting toolbar",
            }),
            serde_json::json!({
                "action": "click",
                "target": "Italic button",
                "scope": "formatting toolbar",
            }),
        ];

        // Run the same 10 steps individually.
        let individual_start = std::time::Instant::now();
        for (i, step) in targets.iter().enumerate() {
            let action = step["action"].as_str().unwrap();
            let tool_name = format!("gui_{action}");
            let mut args = step.clone();
            // Remove the 'action' field — individual tools don't use it.
            args.as_object_mut().unwrap().remove("action");
            args.as_object_mut()
                .unwrap()
                .insert("app".to_string(), serde_json::json!("TextEdit"));
            let result = handler
                .handle(gui_invocation(
                    session.clone(),
                    turn.clone(),
                    &tool_name,
                    args,
                ))
                .await;
            match result {
                Ok(output) => {
                    eprintln!(
                        "[individual step {i}] {action}: success={}, {:.0}ms",
                        output.success,
                        individual_start.elapsed().as_millis()
                    );
                }
                Err(e) => {
                    eprintln!("[individual step {i}] {action}: error={e}");
                }
            }
        }
        let individual_elapsed = individual_start.elapsed();

        // ── B. Reset and run as a batch ──────────────────────────────
        close_textedit_without_saving();
        std::thread::sleep(std::time::Duration::from_millis(600));
        launch_textedit();
        let new_doc2 = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("Cmd+N again");
        assert!(new_doc2.success);
        std::thread::sleep(std::time::Duration::from_millis(800));

        let batch_start = std::time::Instant::now();
        let batch_result = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_batch",
                serde_json::json!({
                    "app": "TextEdit",
                    "steps": targets,
                }),
            ))
            .await;

        let batch_elapsed = batch_start.elapsed();
        match &batch_result {
            Ok(output) => {
                eprintln!(
                    "[batch] success={}, steps_count={}",
                    output.success, output.code_result["steps_count"]
                );
            }
            Err(e) => {
                eprintln!("[batch] error={e}");
            }
        }

        // ── C. Same batch with unified grounding strategy ─────────────
        close_textedit_without_saving();
        std::thread::sleep(std::time::Duration::from_millis(600));
        launch_textedit();
        let new_doc3 = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("Cmd+N for unified test");
        assert!(new_doc3.success);
        std::thread::sleep(std::time::Duration::from_millis(800));

        let unified_start = std::time::Instant::now();
        let unified_result = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_batch",
                serde_json::json!({
                    "app": "TextEdit",
                    "grounding_strategy": "unified",
                    "steps": targets,
                }),
            ))
            .await;
        let unified_elapsed = unified_start.elapsed();
        match &unified_result {
            Ok(output) => {
                eprintln!(
                    "[unified] success={}, steps_count={}",
                    output.success, output.code_result["steps_count"]
                );
            }
            Err(e) => {
                eprintln!("[unified] error={e}");
            }
        }

        let parallel_speedup = individual_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64();
        let unified_speedup = individual_elapsed.as_secs_f64() / unified_elapsed.as_secs_f64();
        eprintln!("=====================================================");
        eprintln!(
            "[TIMING] 10 individual calls:    {:.1}s",
            individual_elapsed.as_secs_f64()
        );
        eprintln!(
            "[TIMING] 10 parallel batch:      {:.1}s  ({:.1}x)",
            batch_elapsed.as_secs_f64(),
            parallel_speedup,
        );
        eprintln!(
            "[TIMING] 10 unified batch:       {:.1}s  ({:.1}x)",
            unified_elapsed.as_secs_f64(),
            unified_speedup,
        );
        eprintln!("=====================================================");
    }
    .await;

    close_textedit_without_saving();
    outcome
}

/// Timing comparison: individual gui_key/gui_type calls vs gui_batch.
/// Measures the wall-clock time difference to validate the speedup.
#[tokio::test]
#[cfg(target_os = "macos")]
#[ignore = "manual macOS GUI smoke test requiring Screen Recording and Accessibility permissions"]
async fn macos_gui_batch_vs_individual_timing() {
    close_textedit_without_saving();
    launch_textedit();

    let handler = GuiHandler::default();
    let (session, turn) = crate::codex::make_session_and_context().await;
    let session = Arc::new(session);
    let turn = Arc::new(turn);

    let outcome = async {
        // ── Individual calls ─────────────────────────────────────────
        let new_doc = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("gui_key Cmd+N");
        assert!(new_doc.success);
        std::thread::sleep(std::time::Duration::from_millis(500));

        let individual_start = std::time::Instant::now();

        handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Individual A",
                }),
            ))
            .await
            .expect("type A");
        handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "Return",
                }),
            ))
            .await
            .expect("Enter");
        handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Individual B",
                    "replace": false,
                }),
            ))
            .await
            .expect("type B");
        handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "Return",
                }),
            ))
            .await
            .expect("Enter 2");
        handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_type",
                serde_json::json!({
                    "app": "TextEdit",
                    "value": "Individual C",
                    "replace": false,
                }),
            ))
            .await
            .expect("type C");

        let individual_elapsed = individual_start.elapsed();
        wait_for_textedit_document_text("Individual A\nIndividual B\nIndividual C");

        // ── Batched calls ────────────────────────────────────────────
        // Close and reopen TextEdit for a clean slate
        close_textedit_without_saving();
        std::thread::sleep(std::time::Duration::from_millis(400));
        launch_textedit();

        let new_doc2 = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_key",
                serde_json::json!({
                    "app": "TextEdit",
                    "key": "n",
                    "modifiers": ["command"],
                }),
            ))
            .await
            .expect("gui_key Cmd+N");
        assert!(new_doc2.success);
        std::thread::sleep(std::time::Duration::from_millis(500));

        let batch_start = std::time::Instant::now();

        let batch_result = handler
            .handle(gui_invocation(
                session.clone(),
                turn.clone(),
                "gui_batch",
                serde_json::json!({
                    "app": "TextEdit",
                    "steps": [
                        { "action": "type", "value": "Batched A" },
                        { "action": "key", "key": "Return" },
                        { "action": "type", "value": "Batched B", "replace": false },
                        { "action": "key", "key": "Return" },
                        { "action": "type", "value": "Batched C", "replace": false },
                    ]
                }),
            ))
            .await
            .expect("gui_batch 5 steps");
        assert!(batch_result.success);

        let batch_elapsed = batch_start.elapsed();
        wait_for_textedit_document_text("Batched A\nBatched B\nBatched C");

        let speedup = individual_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64();
        eprintln!("=====================================================");
        eprintln!("[TIMING] 5 individual calls: {:?}", individual_elapsed);
        eprintln!("[TIMING] 5 batched steps:    {:?}", batch_elapsed);
        eprintln!("[TIMING] Speedup:            {:.1}x", speedup);
        eprintln!("=====================================================");
        assert!(
            speedup > 1.5,
            "Batch should be significantly faster than individual calls, got {speedup:.1}x"
        );
    }
    .await;

    close_textedit_without_saving();
    outcome
}
