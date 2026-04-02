use std::collections::HashMap;
use std::io::BufRead;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;

use crate::function_tool::FunctionCallError;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use super::GUI_UNSUPPORTED_MESSAGE;
use super::HelperCaptureContext;
use super::HelperRect;
use super::ObserveState;
use super::WindowSelector;
use super::readiness::GUI_TOOL_NAMES;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use super::readiness::GuiEnvironmentReadinessCheck;
use super::readiness::GuiEnvironmentReadinessSnapshot;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use super::readiness::GuiReadinessStatus;
use super::readiness::GuiToolCapability;

#[cfg(target_os = "macos")]
pub(super) mod platform_macos;
#[cfg(target_os = "windows")]
pub(super) mod platform_windows;

pub(super) trait GuiPlatform: Send + Sync {
    fn readiness_snapshot(&self) -> GuiEnvironmentReadinessSnapshot;

    fn tool_capabilities(&self) -> HashMap<&'static str, GuiToolCapability> {
        GUI_TOOL_NAMES
            .into_iter()
            .map(|tool_name| {
                (
                    tool_name,
                    GuiToolCapability {
                        enabled: true,
                        reason: None,
                        targetless_only: false,
                    },
                )
            })
            .collect()
    }

    fn resolve_helper_binary(&self) -> Result<PathBuf, FunctionCallError>;

    fn cleanup_input_state(&self) -> Result<(), FunctionCallError> {
        Ok(())
    }

    fn start_emergency_stop_monitor(
        &self,
    ) -> Result<Option<GuiEmergencyStopMonitor>, FunctionCallError> {
        Ok(None)
    }

    fn capture_context(
        &self,
        app: Option<&str>,
        activate_app: bool,
        window_selection: Option<&WindowSelector>,
    ) -> Result<HelperCaptureContext, FunctionCallError>;

    fn capture_region(
        &self,
        bounds: &HelperRect,
        target_width: u32,
        target_height: u32,
    ) -> Result<Vec<u8>, FunctionCallError>;

    fn observe(
        &self,
        app: Option<&str>,
        activate_app: bool,
        capture_mode: Option<&str>,
        window_selection: Option<&WindowSelector>,
        prefer_window_when_available: bool,
    ) -> Result<PlatformObservation, FunctionCallError>;

    fn run_event(
        &self,
        event_mode: &str,
        app: Option<&str>,
        float_env: &[(&str, f64)],
        string_env: &[(&str, String)],
    ) -> Result<(), FunctionCallError>;

    fn run_system_events_type(
        &self,
        app: Option<&str>,
        window_selection: Option<&WindowSelector>,
        text: &str,
        replace: bool,
        submit: bool,
        strategy: &str,
    ) -> Result<(), FunctionCallError>;
}

pub(super) struct PlatformObservation {
    pub(super) state: ObserveState,
    pub(super) image_bytes: Vec<u8>,
}

pub(super) struct GuiEmergencyStopMonitor {
    child: Arc<Mutex<Child>>,
    triggered: Arc<AtomicBool>,
    output_thread: Option<JoinHandle<()>>,
}

impl GuiEmergencyStopMonitor {
    pub(super) fn from_child(mut child: Child) -> Result<Self, FunctionCallError> {
        let stdout = child.stdout.take().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "native GUI emergency stop monitor did not expose stdout".to_string(),
            )
        })?;
        let child = Arc::new(Mutex::new(child));
        let triggered = Arc::new(AtomicBool::new(false));
        let thread_triggered = Arc::clone(&triggered);
        let output_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.trim().eq_ignore_ascii_case("escape") {
                    thread_triggered.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });
        Ok(Self {
            child,
            triggered,
            output_thread: Some(output_thread),
        })
    }

    pub(super) fn triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    pub(super) fn stop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(output_thread) = self.output_thread.take() {
            let _ = output_thread.join();
        }
    }
}

impl Drop for GuiEmergencyStopMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct UnsupportedGuiPlatform;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl GuiPlatform for UnsupportedGuiPlatform {
    fn readiness_snapshot(&self) -> GuiEnvironmentReadinessSnapshot {
        GuiEnvironmentReadinessSnapshot {
            status: "unsupported",
            checks: vec![GuiEnvironmentReadinessCheck {
                id: "platform",
                label: "Platform",
                status: GuiReadinessStatus::Unsupported,
                summary: GUI_UNSUPPORTED_MESSAGE.to_string(),
                detail: None,
            }],
        }
    }

    fn resolve_helper_binary(&self) -> Result<PathBuf, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            GUI_UNSUPPORTED_MESSAGE.to_string(),
        ))
    }

    fn tool_capabilities(&self) -> HashMap<&'static str, GuiToolCapability> {
        GUI_TOOL_NAMES
            .into_iter()
            .map(|tool_name| {
                (
                    tool_name,
                    GuiToolCapability {
                        enabled: false,
                        reason: Some(GUI_UNSUPPORTED_MESSAGE.to_string()),
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
            GUI_UNSUPPORTED_MESSAGE.to_string(),
        ))
    }

    fn capture_region(
        &self,
        _bounds: &HelperRect,
        _target_width: u32,
        _target_height: u32,
    ) -> Result<Vec<u8>, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            GUI_UNSUPPORTED_MESSAGE.to_string(),
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
            GUI_UNSUPPORTED_MESSAGE.to_string(),
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
            GUI_UNSUPPORTED_MESSAGE.to_string(),
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
            GUI_UNSUPPORTED_MESSAGE.to_string(),
        ))
    }
}

pub(super) fn default_gui_platform() -> &'static dyn GuiPlatform {
    #[cfg(target_os = "macos")]
    {
        static PLATFORM: platform_macos::MacOSPlatform = platform_macos::MacOSPlatform;
        return &PLATFORM;
    }

    #[cfg(target_os = "windows")]
    {
        static PLATFORM: platform_windows::WindowsPlatform = platform_windows::WindowsPlatform;
        return &PLATFORM;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        static PLATFORM: UnsupportedGuiPlatform = UnsupportedGuiPlatform;
        &PLATFORM
    }
}

pub(super) fn resolve_gui_platform_tool_capabilities() -> HashMap<&'static str, GuiToolCapability> {
    default_gui_platform().tool_capabilities()
}
