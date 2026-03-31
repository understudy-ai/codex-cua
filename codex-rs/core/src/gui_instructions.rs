pub(crate) fn render_gui_tools_section(gui_tools_enabled: bool) -> Option<String> {
    if !gui_tools_enabled {
        return None;
    }

    Some(
        "## Native GUI Tools\nWhen the experimental `gui_tools` feature is enabled, the built-in macOS GUI tools are available as normal native functions at the same level as other built-in tools.\nUse them as a tight observe-act-verify loop:\n- Start with `gui_observe` to inspect the current GUI and establish the coordinate space for coordinate-based `gui_click`, `gui_drag`, and `gui_scroll`.\n- Prefer semantic targeting first when it is clear: `gui_click.target`, `gui_drag.from_target` plus `gui_drag.to_target`, `gui_type.target`, `gui_scroll.target`, and `gui_wait.target` can resolve visible GUI controls by label or meaning, optionally refined with `location_hint` fields.\n- After a state-changing GUI action, prefer `gui_wait` or a fresh `gui_observe` before the next action so you verify what changed instead of assuming success.\n- Reuse `capture_mode`, `window_title`, or `window_selector` across related GUI steps to keep the tool focused on the same surface.\n- Coordinate-based `gui_click`, `gui_drag`, and `gui_scroll` use the top-left of the most recent observed image as their origin unless you explicitly retarget them.\n- Use `gui_type.secret_env_var` or `gui_type.secret_command_env_var` for sensitive values.\n- Prefer `capture_mode: \"window\"` for in-app work and `capture_mode: \"display\"` for desktop-wide UI such as the Dock, menu bar, permission prompts, or cross-window drags."
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
        assert!(rendered.contains("`gui_wait`"));
        assert!(rendered.contains("`gui_drag.from_target`"));
        assert!(rendered.contains("observe-act-verify"));
        assert!(rendered.contains("semantic targeting"));
    }
}
