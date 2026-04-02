use std::path::PathBuf;

use crate::function_tool::FunctionCallError;

use super::super::HelperCaptureContext;
use super::super::HelperRect;
use super::super::WindowSelector;
use super::super::readiness::GUI_TOOL_NAMES;
use super::super::readiness::GuiEnvironmentReadinessCheck;
use super::super::readiness::GuiEnvironmentReadinessSnapshot;
use super::super::readiness::GuiReadinessStatus;
use super::super::readiness::GuiToolCapability;
use super::GuiPlatform;
use super::PlatformObservation;

const WINDOWS_GUI_UNIMPLEMENTED_MESSAGE: &str =
    "Windows native GUI backend is not implemented yet.";

pub(super) struct WindowsPlatform;

impl GuiPlatform for WindowsPlatform {
    fn readiness_snapshot(&self) -> GuiEnvironmentReadinessSnapshot {
        GuiEnvironmentReadinessSnapshot {
            status: "unsupported",
            checks: vec![GuiEnvironmentReadinessCheck {
                id: "platform",
                label: "Platform",
                status: GuiReadinessStatus::Unsupported,
                summary: WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
                detail: Some(
                    "Reserved backend slot for a future Windows implementation built around screenshot capture and native input dispatch."
                        .to_string(),
                ),
            }],
        }
    }

    fn resolve_helper_binary(&self) -> Result<PathBuf, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
        ))
    }

    fn tool_capabilities(&self) -> std::collections::HashMap<&'static str, GuiToolCapability> {
        GUI_TOOL_NAMES
            .into_iter()
            .map(|tool_name| {
                (
                    tool_name,
                    GuiToolCapability {
                        enabled: false,
                        reason: Some(WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string()),
                        targetless_only: false,
                    },
                )
            })
            .collect()
    }

    fn capture_context(
        &self,
        _app: Option<&str>,
        _activate_app: bool,
        _window_selection: Option<&WindowSelector>,
    ) -> Result<HelperCaptureContext, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
        ))
    }

    fn capture_region(
        &self,
        _bounds: &HelperRect,
        _target_width: u32,
        _target_height: u32,
    ) -> Result<Vec<u8>, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
        ))
    }

    fn run_event(
        &self,
        _event_mode: &str,
        _app: Option<&str>,
        _float_env: &[(&str, f64)],
        _string_env: &[(&str, String)],
    ) -> Result<(), FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
        ))
    }

    fn observe(
        &self,
        _app: Option<&str>,
        _activate_app: bool,
        _capture_mode: Option<&str>,
        _window_selection: Option<&WindowSelector>,
        _prefer_window_when_available: bool,
    ) -> Result<PlatformObservation, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
        ))
    }

    fn run_system_events_type(
        &self,
        _app: Option<&str>,
        _window_selection: Option<&WindowSelector>,
        _text: &str,
        _replace: bool,
        _submit: bool,
        _strategy: &str,
    ) -> Result<(), FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            WINDOWS_GUI_UNIMPLEMENTED_MESSAGE.to_string(),
        ))
    }
}
