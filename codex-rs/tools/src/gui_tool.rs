use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use std::collections::BTreeMap;

fn string_enum_description(values: &[&str], extra: &str) -> String {
    format!("Supported values: {}. {extra}", values.join(", "))
}

fn window_selector_schema() -> JsonSchema {
    JsonSchema::Object {
        properties: BTreeMap::from([
            (
                "title".to_string(),
                JsonSchema::String {
                    description: Some("Optional exact visible window title.".to_string()),
                },
            ),
            (
                "title_contains".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional visible substring of the target window title.".to_string(),
                    ),
                },
            ),
            (
                "index".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Optional 1-based index among matching visible windows.".to_string(),
                    ),
                },
            ),
        ]),
        required: None,
        additional_properties: Some(false.into()),
    }
}

fn with_capture_selection_properties(
    mut properties: BTreeMap<String, JsonSchema>,
    include_window_title: bool,
) -> BTreeMap<String, JsonSchema> {
    properties.insert(
        "capture_mode".to_string(),
        JsonSchema::String {
            description: Some(string_enum_description(
                &["display", "window"],
                "Use `window` to capture the active app window when available. When omitted, GUI tools prefer `window` for in-app or window-targeted work and fall back to `display` otherwise.",
            )),
        },
    );
    if include_window_title {
        properties.insert(
            "window_title".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional exact visible window title to capture or focus.".to_string(),
                ),
            },
        );
    }
    properties.insert("window_selector".to_string(), window_selector_schema());
    properties
}

fn with_post_action_evidence_properties(
    mut properties: BTreeMap<String, JsonSchema>,
) -> BTreeMap<String, JsonSchema> {
    properties.insert(
        "post_action_settle_ms".to_string(),
        JsonSchema::Number {
            description: Some(
                "How long to wait before capturing post-action evidence. Defaults vary by tool."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "return_image".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "Whether to attach a post-action evidence screenshot. Defaults to true when image inputs are supported."
                    .to_string(),
            ),
        },
    );
    properties
}

fn with_target_properties(
    mut properties: BTreeMap<String, JsonSchema>,
    action_description: &str,
) -> BTreeMap<String, JsonSchema> {
    properties.insert(
        "target".to_string(),
        JsonSchema::String {
            description: Some(format!(
                "Optional semantic GUI target to resolve before {action_description}, such as `Save button`, `Search field`, or `Sidebar`."
            )),
        },
    );
    properties.insert(
        "location_hint".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional disambiguation hint such as `top right`, `left sidebar`, or `near the bottom`."
                    .to_string(),
            ),
        },
    );
    properties
}

fn with_drag_target_properties(
    mut properties: BTreeMap<String, JsonSchema>,
) -> BTreeMap<String, JsonSchema> {
    properties.insert(
        "from_target".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional semantic GUI drag source to resolve before dragging, such as `Selected tab`, `Message row`, or `Resize handle`."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "from_location_hint".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional disambiguation hint for `from_target`, such as `left sidebar` or `near the top`."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "to_target".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional semantic GUI drag destination to resolve before dragging, such as `Trash`, `Calendar column`, or `right pane`."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "to_location_hint".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional disambiguation hint for `to_target`, such as `bottom right` or `in the center`."
                    .to_string(),
            ),
        },
    );
    properties
}

pub fn create_gui_observe_tool() -> ToolSpec {
    let properties = with_capture_selection_properties(BTreeMap::from([
        (
            "app".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional macOS application name to activate before capturing. Defaults to the current frontmost app."
                        .to_string(),
                ),
            },
        ),
        (
            "return_image".to_string(),
            JsonSchema::Boolean {
                description: Some(
                    "Whether to attach the captured screenshot image. Defaults to true."
                        .to_string(),
                ),
            },
        ),
    ]), true);

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_observe".to_string(),
        description: "Capture a screenshot of the current macOS GUI. Use this before GUI actions to inspect state and obtain the coordinate space for other gui_* tools. Supports display-wide capture and focused-window capture."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_wait_tool() -> ToolSpec {
    let properties = with_target_properties(with_capture_selection_properties(
        BTreeMap::from([
            (
                "duration_ms".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "How long to wait before refreshing the GUI screenshot. Defaults to 1000."
                            .to_string(),
                    ),
                },
            ),
            (
                "state".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &["appear", "disappear"],
                        "Only used when `target` is provided. Defaults to `appear`.",
                    )),
                },
            ),
            (
                "timeout_ms".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Maximum time to wait for `target` to satisfy `state`. Only used when `target` is provided. Defaults to 5000."
                            .to_string(),
                    ),
                },
            ),
            (
                "interval_ms".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Polling interval between semantic target checks. Only used when `target` is provided. Defaults to 500."
                            .to_string(),
                    ),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional macOS application name to activate before refreshing the screenshot. Defaults to the current frontmost app."
                            .to_string(),
                    ),
                },
            ),
            (
                "return_image".to_string(),
                JsonSchema::Boolean {
                    description: Some(
                        "Whether to attach the refreshed screenshot image. Defaults to true."
                            .to_string(),
                    ),
                },
            ),
        ]),
        true,
    ), "waiting for a semantic GUI target");

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_wait".to_string(),
        description: "Wait briefly, then refresh the current macOS GUI screenshot so you can verify the next state after a GUI action. Reuses the previous gui_observe target when no explicit capture selection is provided, and can also wait for a semantic GUI target to appear or disappear."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_click_tool() -> ToolSpec {
    let properties = with_target_properties(
        with_post_action_evidence_properties(with_capture_selection_properties(
            BTreeMap::from([
                (
                    "x".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Horizontal pixel coordinate measured from the top-left of the most recent gui_observe image."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "y".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Vertical pixel coordinate measured from the top-left of the most recent gui_observe image."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "button".to_string(),
                    JsonSchema::String {
                        description: Some(string_enum_description(
                            &["left", "right", "none"],
                            "Use `none` for hover-only pointer movement. Defaults to `left`.",
                        )),
                    },
                ),
                (
                    "clicks".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Number of clicks to send. Defaults to 1. Use 2 for a double-click."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "hold_ms".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional press-and-hold duration in milliseconds before releasing. Use this for long-press interactions."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "settle_ms".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional hover settle time in milliseconds when `button` is `none`. Defaults to 200."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "app".to_string(),
                    JsonSchema::String {
                        description: Some(
                            "Optional macOS application name to activate before clicking."
                                .to_string(),
                        ),
                    },
                ),
            ]),
            true,
        )),
        "clicking or hovering",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_click".to_string(),
        description:
            "Click, right-click, double-click, hover, or click-and-hold at a coordinate in the current macOS GUI. Coordinates are interpreted in the coordinate space returned by gui_observe, or you can provide `target` to resolve a semantic control first."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_drag_tool() -> ToolSpec {
    let properties = with_drag_target_properties(with_post_action_evidence_properties(
        with_capture_selection_properties(
            BTreeMap::from([
                (
                    "from_x".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional horizontal drag start coordinate measured from the top-left of the most recent gui_observe image."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "from_y".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional vertical drag start coordinate measured from the top-left of the most recent gui_observe image."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "to_x".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional horizontal drag destination coordinate measured from the top-left of the most recent gui_observe image."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "to_y".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional vertical drag destination coordinate measured from the top-left of the most recent gui_observe image."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "duration_ms".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional drag duration in milliseconds. Defaults to 450."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "steps".to_string(),
                    JsonSchema::Number {
                        description: Some(
                            "Optional number of interpolation steps. Defaults to 24."
                                .to_string(),
                        ),
                    },
                ),
                (
                    "app".to_string(),
                    JsonSchema::String {
                        description: Some(
                            "Optional macOS application name to activate before dragging."
                                .to_string(),
                        ),
                    },
                ),
            ]),
            true,
        ),
    ));

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_drag".to_string(),
        description:
            "Drag between two points in the current macOS GUI. You can provide semantic `from_target` and `to_target`, or fall back to coordinate pairs measured in the coordinate space returned by gui_observe."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_scroll_tool() -> ToolSpec {
    let properties = with_target_properties(
        with_post_action_evidence_properties(with_capture_selection_properties(BTreeMap::from([
            (
                "delta_y".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Vertical scroll amount. Positive values scroll down; negative values scroll up."
                            .to_string(),
                    ),
                },
            ),
            (
                "delta_x".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Horizontal scroll amount. Positive values scroll right; negative values scroll left."
                            .to_string(),
                    ),
                },
            ),
            (
                "x".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Optional horizontal pixel coordinate to move the cursor to before scrolling, measured from the top-left of the most recent gui_observe image."
                            .to_string(),
                    ),
                },
            ),
            (
                "y".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Optional vertical pixel coordinate to move the cursor to before scrolling, measured from the top-left of the most recent gui_observe image."
                            .to_string(),
                    ),
                },
            ),
            (
                "unit".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &["line", "pixel"],
                        "Defaults to `line`.",
                    )),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional macOS application name to activate before scrolling."
                            .to_string(),
                    ),
                },
            ),
        ]), true)),
        "scrolling",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_scroll".to_string(),
        description: "Scroll in the current macOS GUI. Provide at least one of delta_x or delta_y, and optionally supply `target` to scroll a semantic region without manual coordinates."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_type_tool() -> ToolSpec {
    let properties = with_target_properties(
        with_post_action_evidence_properties(with_capture_selection_properties(BTreeMap::from([
            (
                "text".to_string(),
                JsonSchema::String {
                    description: Some("Literal text to type into the currently focused control."
                        .to_string()),
                },
            ),
            (
                "secret_env_var".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Environment variable name whose value should be typed without exposing the literal secret in the tool call."
                            .to_string(),
                    ),
                },
            ),
            (
                "secret_command_env_var".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Environment variable name containing a local shell command whose stdout should be typed without exposing the secret in the tool call."
                            .to_string(),
                    ),
                },
            ),
            (
                "replace".to_string(),
                JsonSchema::Boolean {
                    description: Some(
                        "Whether to replace the current field contents before typing. Defaults to true."
                            .to_string(),
                    ),
                },
            ),
            (
                "submit".to_string(),
                JsonSchema::Boolean {
                    description: Some(
                        "Whether to press Return after typing. Defaults to false.".to_string(),
                    ),
                },
            ),
            (
                "strategy".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &[
                            "unicode",
                            "clipboard_paste",
                            "physical_keys",
                            "system_events_paste",
                            "system_events_keystroke",
                            "system_events_keystroke_chars",
                        ],
                        "Defaults to `unicode`.",
                    )),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional macOS application name to activate before typing."
                            .to_string(),
                    ),
                },
            ),
        ]), true)),
        "typing into a semantic input target",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_type".to_string(),
        description:
            "Type text into the currently focused macOS GUI control. Typically use gui_click first to focus the desired field, or provide `target` so the tool focuses the semantic input target for you. Provide exactly one of `text`, `secret_env_var`, or `secret_command_env_var`."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: None,
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_key_tool() -> ToolSpec {
    let properties = with_post_action_evidence_properties(with_capture_selection_properties(
        BTreeMap::from([
            (
                "key".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Key to press, such as `Enter`, `Tab`, `Escape`, `ArrowDown`, or `s`."
                            .to_string(),
                    ),
                },
            ),
            (
                "modifiers".to_string(),
                JsonSchema::Array {
                    items: Box::new(JsonSchema::String {
                        description: Some(
                            "Modifier name such as `command`, `shift`, `option`, or `control`."
                                .to_string(),
                        ),
                    }),
                    description: Some("Optional modifier list.".to_string()),
                },
            ),
            (
                "repeat".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "How many times to press the key. Defaults to 1.".to_string(),
                    ),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional macOS application name to activate before pressing the key."
                            .to_string(),
                    ),
                },
            ),
        ]),
        true,
    ));

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_key".to_string(),
        description: "Press a key or hotkey in the current macOS GUI.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["key".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_move_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "x".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Absolute macOS display X coordinate in logical points.".to_string(),
                ),
            },
        ),
        (
            "y".to_string(),
            JsonSchema::Number {
                description: Some(
                    "Absolute macOS display Y coordinate in logical points.".to_string(),
                ),
            },
        ),
        (
            "app".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional macOS application name to activate before moving the pointer."
                        .to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_move".to_string(),
        description: "Move the macOS pointer to an absolute display coordinate in logical points."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["x".to_string(), "y".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function_parameters(tool: ToolSpec) -> BTreeMap<String, JsonSchema> {
        match tool {
            ToolSpec::Function(tool) => match tool.parameters {
                JsonSchema::Object { properties, .. } => properties,
                schema => panic!("expected object schema, got {schema:?}"),
            },
            other => panic!("expected function tool, got {other:?}"),
        }
    }

    #[test]
    fn click_tool_exposes_semantic_target_properties() {
        let properties = function_parameters(create_gui_click_tool());

        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("location_hint"));
    }

    #[test]
    fn wait_tool_exposes_target_wait_controls() {
        let properties = function_parameters(create_gui_wait_tool());

        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("state"));
        assert!(properties.contains_key("timeout_ms"));
        assert!(properties.contains_key("interval_ms"));
    }

    #[test]
    fn drag_tool_exposes_semantic_source_and_destination_properties() {
        let properties = function_parameters(create_gui_drag_tool());

        assert!(properties.contains_key("from_target"));
        assert!(properties.contains_key("from_location_hint"));
        assert!(properties.contains_key("to_target"));
        assert!(properties.contains_key("to_location_hint"));
    }
}
