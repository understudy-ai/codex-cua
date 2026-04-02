pub(crate) fn render_gui_tools_section(
    gui_tools_enabled: bool,
    gui_coordinate_targeting: bool,
) -> Option<String> {
    if !gui_tools_enabled {
        return None;
    }

    let coordinate_guidance = if gui_coordinate_targeting {
        "\n- Coordinate-based `gui_click` and `gui_drag` may appear when `[tools.gui] coordinate_targeting = true`, but that path is currently an experimental placeholder and is disabled by default.\n- Prefer semantic grounding for real work; the direct-coordinate path is reserved for future experiments after its benchmark quality improves."
    } else {
        ""
    };

    Some(format!(
        "## Native GUI Tools\nWhen the experimental `gui_tools` feature is enabled, the built-in macOS GUI tools are available as normal native functions at the same level as other built-in tools.\nUse them as a tight observe-act-verify loop:\n- Start with `gui_observe` to inspect the current GUI surface and optionally ground a semantic control without acting yet.\n- Prefer semantic targeting first when it is clear: `gui_observe.target`, `gui_click.target`, `gui_drag.from_target` plus `gui_drag.to_target`, `gui_type.target`, `gui_scroll.target`, and `gui_wait.target` resolve visible GUI controls by label or meaning, optionally refined with `location_hint`, `scope`, and `grounding_mode`.\n- Write GUI `target` descriptions using visible screenshot evidence: prefer the exact on-screen text, icon, state, nearby context, and coarse location that make the control unique.\n- Name the actionable or editable surface itself in `target`, not surrounding whitespace, generic container chrome, or descriptive text next to the real control.\n- When several similar controls are visible, add nearby text, state, relative position, `scope`, or `window_title` so the runtime can disambiguate them.\n- `gui_key.key` must be a plain literal key name like `Enter`, `Escape`, or a single printable character. Do not wrap the key value in markdown backticks or extra punctuation.\n- After a state-changing GUI action, prefer `gui_wait` or a fresh `gui_observe` before the next action so you verify what changed instead of assuming success.\n- If a GUI action misses, ground again from a fresh screenshot instead of reusing stale assumptions about the old surface.\n- After one failed attempt on the same visible target, revise the `target` and `scope` using the latest screenshot evidence and prefer `grounding_mode: \"complex\"` for the retry.\n- Reuse `capture_mode`, `window_title`, or `window_selector` across related GUI steps to keep the tool focused on the same surface.\n- Use `gui_type.secret_env_var` or `gui_type.secret_command_env_var` for sensitive values.\n- Prefer `capture_mode: \"window\"` for in-app work and `capture_mode: \"display\"` for desktop-wide UI such as the Dock, menu bar, permission prompts, or cross-window drags.\n- Remember that GUI actions now run with native safety guards: avoid overlapping risky actions, and stop to re-observe when the UI looks different than expected.{coordinate_guidance}"
    ))
}

#[cfg(test)]
mod tests {
    use super::render_gui_tools_section;

    #[test]
    fn omits_gui_tools_section_when_disabled() {
        assert_eq!(render_gui_tools_section(false, false), None);
    }

    #[test]
    fn renders_gui_tools_section_when_enabled() {
        let rendered = render_gui_tools_section(true, false).expect("gui instructions");

        assert!(rendered.contains("## Native GUI Tools"));
        assert!(rendered.contains("`gui_observe.target`"));
        assert!(rendered.contains("`gui_wait`"));
        assert!(rendered.contains("`gui_drag.from_target`"));
        assert!(rendered.contains("`scope`"));
        assert!(rendered.contains("observe-act-verify"));
        assert!(rendered.contains("semantic targeting"));
        assert!(rendered.contains("visible screenshot evidence"));
        assert!(rendered.contains("grounding_mode: \"complex\""));
        assert!(!rendered.contains("Coordinate-based `gui_click`"));
    }

    #[test]
    fn renders_coordinate_guidance_when_enabled() {
        let rendered = render_gui_tools_section(true, true).expect("gui instructions");

        assert!(rendered.contains("Coordinate-based `gui_click` and `gui_drag`"));
        assert!(rendered.contains("experimental placeholder"));
        assert!(rendered.contains("disabled by default"));
        assert!(rendered.contains("Prefer semantic grounding"));
    }
}
