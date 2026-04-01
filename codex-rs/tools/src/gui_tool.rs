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
    properties.insert(
        "scope".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional semantic region that further narrows the target, such as `left sidebar`, `toolbar`, or `composer pane`."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "grounding_mode".to_string(),
        JsonSchema::String {
            description: Some(string_enum_description(
                &["single", "complex"],
                "Optional grounding hint. `single` suits simple isolated controls; `complex` enables the heavier validation and retry path for dense or ambiguous layouts.",
            )),
        },
    );
    properties
}

fn with_drag_target_properties(
    mut properties: BTreeMap<String, JsonSchema>,
) -> BTreeMap<String, JsonSchema> {
    properties.insert(
        "grounding_mode".to_string(),
        JsonSchema::String {
            description: Some(string_enum_description(
                &["single", "complex"],
                "Optional grounding hint shared by drag source and destination. `single` suits simple isolated controls; `complex` enables the heavier validation and retry path for dense or ambiguous layouts.",
            )),
        },
    );
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
        "from_scope".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional semantic region that narrows `from_target`, such as `left sidebar` or `active tab strip`."
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
    properties.insert(
        "to_scope".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional semantic region that narrows `to_target`, such as `timeline`, `calendar grid`, or `drop zone`."
                    .to_string(),
            ),
        },
    );
    properties
}

pub fn create_gui_observe_tool() -> ToolSpec {
    let properties = with_target_properties(
        with_capture_selection_properties(
            BTreeMap::from([
                (
                    "app".to_string(),
                    JsonSchema::String {
                        description: Some(
                            "Optional application name to activate before capturing. Defaults to the current frontmost app."
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
            ]),
            true,
        ),
        "observing a semantic GUI target",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_observe".to_string(),
        description: "Capture a screenshot of the current native GUI surface for visual inspection and follow-up GUI grounding. Supports display-wide capture and focused-window capture, and can also resolve a semantic `target` within the observed GUI."
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
                "state".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &["appear", "disappear"],
                        "Defaults to `appear`.",
                    )),
                },
            ),
            (
                "timeout_ms".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Maximum time to wait for `target` to satisfy `state`. Defaults to 8000."
                            .to_string(),
                    ),
                },
            ),
            (
                "interval_ms".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Polling interval between semantic target checks. Defaults to 350."
                            .to_string(),
                    ),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional application name to activate before refreshing the screenshot. Defaults to the current frontmost app."
                            .to_string(),
                    ),
                },
            ),
        ]),
        true,
    ), "waiting for a semantic GUI target");

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_wait".to_string(),
        description: "Repeatedly refresh the current native GUI screenshot until a semantic target appears or disappears. Uses consecutive confirmations for stability and reuses the previous gui_observe capture selection when no explicit capture selection is provided."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["target".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_click_tool() -> ToolSpec {
    let properties = with_target_properties(
        with_capture_selection_properties(
            BTreeMap::from([
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
                            "Optional application name to activate before clicking."
                                .to_string(),
                        ),
                    },
                ),
            ]),
            true,
        ),
        "clicking or hovering",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_click".to_string(),
        description:
            "Click, right-click, double-click, hover, or click-and-hold on a semantic target in the current native GUI."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["target".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_drag_tool() -> ToolSpec {
    let properties = with_drag_target_properties(with_capture_selection_properties(
        BTreeMap::from([
            (
                "duration_ms".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Optional drag duration in milliseconds. Defaults to 450.".to_string(),
                    ),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional application name to activate before dragging.".to_string(),
                    ),
                },
            ),
        ]),
        true,
    ));

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_drag".to_string(),
        description:
            "Drag between semantic `from_target` and `to_target` points in the current native GUI."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["from_target".to_string(), "to_target".to_string()]),
            additional_properties: Some(false.into()),
        },
        output_schema: None,
    })
}

pub fn create_gui_scroll_tool() -> ToolSpec {
    let properties = with_target_properties(
        with_capture_selection_properties(BTreeMap::from([
            (
                "direction".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &["up", "down", "left", "right"],
                        "Scroll direction. Defaults to `down`.",
                    )),
                },
            ),
            (
                "distance".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &["small", "medium", "page"],
                        "Semantic scroll distance. Defaults to `page` for targetless scrolls and `medium` for grounded scrolls.",
                    )),
                },
            ),
            (
                "amount".to_string(),
                JsonSchema::Number {
                    description: Some(
                        "Optional explicit legacy line-count override. When provided, it overrides `distance`."
                            .to_string(),
                    ),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional application name to activate before scrolling."
                            .to_string(),
                    ),
                },
            ),
        ]), true),
        "scrolling",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_scroll".to_string(),
        description: "Scroll in the current native GUI. Defaults to a targetless scroll on the current surface, or provide `target` to scroll a semantic region. Prefer `direction` with semantic `distance`; use `amount` only when you need an explicit legacy line count."
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
        with_capture_selection_properties(BTreeMap::from([
            (
                "value".to_string(),
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
                "type_strategy".to_string(),
                JsonSchema::String {
                    description: Some(string_enum_description(
                        &[
                            "clipboard_paste",
                            "physical_keys",
                            "system_events_paste",
                            "system_events_keystroke",
                            "system_events_keystroke_chars",
                        ],
                        "When omitted, the runtime chooses the default native typing path.",
                    )),
                },
            ),
            (
                "app".to_string(),
                JsonSchema::String {
                    description: Some(
                        "Optional application name to activate before typing."
                            .to_string(),
                    ),
                },
            ),
        ]), true),
        "typing into a semantic input target",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_type".to_string(),
        description:
            "Type text into the currently focused native GUI control. Typically use gui_click first to focus the desired field, or provide `target` so the tool focuses the semantic input target for you. Provide exactly one of `value`, `secret_env_var`, or `secret_command_env_var`."
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
    let properties = with_capture_selection_properties(
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
                        "Optional application name to activate before pressing the key."
                            .to_string(),
                    ),
                },
            ),
        ]),
        true,
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_key".to_string(),
        description: "Press a key or hotkey in the current native GUI.".to_string(),
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
                description: Some("Absolute display X coordinate in logical points.".to_string()),
            },
        ),
        (
            "y".to_string(),
            JsonSchema::Number {
                description: Some("Absolute display Y coordinate in logical points.".to_string()),
            },
        ),
        (
            "app".to_string(),
            JsonSchema::String {
                description: Some(
                    "Optional application name to activate before moving the pointer.".to_string(),
                ),
            },
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "gui_move".to_string(),
        description: "Move the pointer to an absolute display coordinate in logical points."
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
        let tool = create_gui_click_tool();
        let properties = function_parameters(tool.clone());

        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("location_hint"));
        assert!(properties.contains_key("scope"));
        assert!(properties.contains_key("grounding_mode"));
        assert!(!properties.contains_key("x"));
        assert!(!properties.contains_key("y"));
        match tool {
            ToolSpec::Function(tool) => {
                let JsonSchema::Object { required, .. } = tool.parameters else {
                    panic!("expected object schema");
                };
                assert_eq!(required, Some(vec!["target".to_string()]));
            }
            other => panic!("expected function tool, got {other:?}"),
        }
    }

    #[test]
    fn wait_tool_exposes_target_wait_controls() {
        let tool = create_gui_wait_tool();
        let properties = function_parameters(tool.clone());

        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("state"));
        assert!(properties.contains_key("timeout_ms"));
        assert!(properties.contains_key("interval_ms"));
        assert!(properties.contains_key("scope"));
        assert!(properties.contains_key("grounding_mode"));
        assert!(!properties.contains_key("duration_ms"));
        assert!(!properties.contains_key("return_image"));
        match tool {
            ToolSpec::Function(tool) => {
                let JsonSchema::Object { required, .. } = tool.parameters else {
                    panic!("expected object schema");
                };
                assert_eq!(required, Some(vec!["target".to_string()]));
            }
            other => panic!("expected function tool, got {other:?}"),
        }
    }

    #[test]
    fn drag_tool_exposes_semantic_source_and_destination_properties() {
        let tool = create_gui_drag_tool();
        let properties = function_parameters(tool.clone());

        assert!(properties.contains_key("grounding_mode"));
        assert!(properties.contains_key("from_target"));
        assert!(properties.contains_key("from_location_hint"));
        assert!(properties.contains_key("from_scope"));
        assert!(properties.contains_key("to_target"));
        assert!(properties.contains_key("to_location_hint"));
        assert!(properties.contains_key("to_scope"));
        assert!(!properties.contains_key("steps"));
        assert!(!properties.contains_key("from_x"));
        assert!(!properties.contains_key("from_y"));
        assert!(!properties.contains_key("to_x"));
        assert!(!properties.contains_key("to_y"));
        match tool {
            ToolSpec::Function(tool) => {
                let JsonSchema::Object { required, .. } = tool.parameters else {
                    panic!("expected object schema");
                };
                assert_eq!(
                    required,
                    Some(vec!["from_target".to_string(), "to_target".to_string()])
                );
            }
            other => panic!("expected function tool, got {other:?}"),
        }
    }

    #[test]
    fn observe_tool_exposes_semantic_grounding_properties() {
        let properties = function_parameters(create_gui_observe_tool());

        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("location_hint"));
        assert!(properties.contains_key("scope"));
        assert!(properties.contains_key("grounding_mode"));
    }

    #[test]
    fn scroll_tool_exposes_understudy_aligned_scroll_semantics() {
        let properties = function_parameters(create_gui_scroll_tool());

        assert!(properties.contains_key("direction"));
        assert!(properties.contains_key("distance"));
        assert!(properties.contains_key("amount"));
        assert!(!properties.contains_key("delta_x"));
        assert!(!properties.contains_key("delta_y"));
        assert!(!properties.contains_key("unit"));
        assert!(!properties.contains_key("x"));
        assert!(!properties.contains_key("y"));
    }

    #[test]
    fn type_tool_matches_understudy_native_contract_shape() {
        let properties = function_parameters(create_gui_type_tool());

        assert!(properties.contains_key("value"));
        assert!(properties.contains_key("type_strategy"));
        assert!(!properties.contains_key("text"));
        assert!(!properties.contains_key("strategy"));
        let JsonSchema::String { description } = &properties["type_strategy"] else {
            panic!("expected string schema");
        };
        let description = description.as_deref().unwrap_or_default();
        assert!(!description.contains("unicode"));
    }
}
