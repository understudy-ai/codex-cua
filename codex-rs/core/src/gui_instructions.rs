pub(crate) fn render_gui_tools_section(gui_tools_enabled: bool) -> Option<String> {
    if !gui_tools_enabled {
        return None;
    }

    Some(
        "## Native GUI Tools\nWhen the experimental `gui_tools` feature is enabled, the built-in macOS GUI tools are available as normal native functions at the same level as other built-in tools.\nUse them as a tight observe-act-verify loop:\n- Start with `gui_observe` to inspect the current GUI surface and optionally ground a semantic control without acting yet.\n- Prefer semantic targeting first when it is clear: `gui_observe.target`, `gui_click.target`, `gui_drag.from_target` plus `gui_drag.to_target`, `gui_type.target`, `gui_scroll.target`, and `gui_wait.target` resolve visible GUI controls by label or meaning, optionally refined with `location_hint`, `scope`, and `grounding_mode`.\n- Treat `gui_click` and `gui_drag` as semantic-only actions. If you need absolute pointer motion, use `gui_move`; if you need targetless scrolling, use `gui_scroll` without `target`.\n- `gui_key.key` must be a plain literal key name like `Enter`, `Escape`, or a single printable character. Do not wrap the key value in markdown backticks or extra punctuation.\n- After a state-changing GUI action, prefer `gui_wait` or a fresh `gui_observe` before the next action so you verify what changed instead of assuming success.\n- Reuse `capture_mode`, `window_title`, or `window_selector` across related GUI steps to keep the tool focused on the same surface.\n- Use `gui_type.secret_env_var` or `gui_type.secret_command_env_var` for sensitive values.\n- Prefer `capture_mode: \"window\"` for in-app work and `capture_mode: \"display\"` for desktop-wide UI such as the Dock, menu bar, permission prompts, or cross-window drags.\n- Remember that GUI actions now run with native safety guards: avoid overlapping risky actions, and stop to re-observe when the UI looks different than expected."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::render_gui_tools_section;

    #[test]
    fn omits_gui_tools_section_when_disabled() {
        assert_eq!(render_gui_tools_section(false), None);
    }

    #[test]
    fn renders_gui_tools_section_when_enabled() {
        let rendered = render_gui_tools_section(true).expect("gui instructions");

        assert!(rendered.contains("## Native GUI Tools"));
        assert!(rendered.contains("`gui_observe.target`"));
        assert!(rendered.contains("`gui_wait`"));
        assert!(rendered.contains("`gui_drag.from_target`"));
        assert!(rendered.contains("`scope`"));
        assert!(rendered.contains("observe-act-verify"));
        assert!(rendered.contains("semantic targeting"));
        assert!(rendered.contains("semantic-only actions"));
        assert!(!rendered.contains("Coordinate-based `gui_click`"));
    }
}
