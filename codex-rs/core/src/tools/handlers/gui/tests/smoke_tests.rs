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
