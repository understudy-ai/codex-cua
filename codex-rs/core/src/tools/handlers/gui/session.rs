use serde::Deserialize;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;

use super::platform::GuiEmergencyStopMonitor;
use super::platform::default_gui_platform;

pub(super) struct GuiActionSession {
    conversation_id: String,
    process_id: u32,
    lock: Option<FileGuiPhysicalResourceLock>,
    emergency_stop_monitor: Option<GuiEmergencyStopMonitor>,
    /// PIDs of applications hidden at the start of the action via
    /// [`hide_other_apps`].  Restored in [`Drop`].
    hidden_pids: Vec<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GuiPhysicalResourceLockHolder {
    session_id: Option<String>,
    pid: Option<u32>,
    acquired_at: Option<u64>,
    tool_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct GuiPhysicalResourceLockPayload<'a> {
    session_id: &'a str,
    pid: u32,
    acquired_at: u64,
    tool_name: &'a str,
}

struct FileGuiPhysicalResourceLock {
    path: PathBuf,
}

impl FileGuiPhysicalResourceLock {
    fn acquire(
        path: PathBuf,
        session_id: &str,
        process_id: u32,
        tool_name: &str,
    ) -> Result<Self, FunctionCallError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                FunctionCallError::RespondToModel(format!(
                    "failed to prepare GUI lock directory: {error}"
                ))
            })?;
        }

        let payload = serde_json::to_vec(&GuiPhysicalResourceLockPayload {
            session_id,
            pid: process_id,
            acquired_at: unix_timestamp_ms(),
            tool_name,
        })
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "failed to encode GUI physical lock payload: {error}"
            ))
        })?;

        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50 * (1 << attempt.min(3))));
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&payload).map_err(|error| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to write GUI physical lock: {error}"
                        ))
                    })?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    let holder = read_lock_holder(&path);
                    if holder
                        .as_ref()
                        .and_then(|holder| holder.session_id.as_deref())
                        == Some(session_id)
                        && holder.as_ref().and_then(|holder| holder.pid) == Some(process_id)
                    {
                        return Ok(Self { path });
                    }
                    if !is_process_alive(holder.as_ref().and_then(|holder| holder.pid)) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    return Err(FunctionCallError::RespondToModel(format!(
                        "GUI physical resources are currently locked by {}.",
                        format_lock_holder(holder.as_ref())
                    )));
                }
                Err(error) => {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "failed to acquire GUI physical lock: {error}"
                    )));
                }
            }
        }

        Err(FunctionCallError::RespondToModel(
            "GUI physical resources are currently locked by another GUI session.".to_string(),
        ))
    }

    fn release(&self, session_id: &str, process_id: u32) {
        let holder = read_lock_holder(&self.path);
        if holder
            .as_ref()
            .and_then(|holder| holder.session_id.as_deref())
            != Some(session_id)
            || holder.as_ref().and_then(|holder| holder.pid) != Some(process_id)
        {
            return;
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl GuiActionSession {
    pub(super) fn throw_if_emergency_stopped(&self) -> Result<(), FunctionCallError> {
        if self
            .emergency_stop_monitor
            .as_ref()
            .is_some_and(GuiEmergencyStopMonitor::triggered)
        {
            return Err(FunctionCallError::RespondToModel(
                "Stopped the GUI action after Escape was pressed.".to_string(),
            ));
        }
        Ok(())
    }

    /// Hide all visible applications except the target app and the host
    /// terminal so that the GUI action operates on a clean screen.  Hidden
    /// PIDs are recorded and automatically restored when this session is
    /// dropped.
    pub(super) fn hide_other_apps(&mut self, app: Option<&str>) {
        if let Ok(pids) = default_gui_platform().hide_other_apps(app) {
            self.hidden_pids = pids;
        }
    }
}

impl Drop for GuiActionSession {
    fn drop(&mut self) {
        if let Some(monitor) = &mut self.emergency_stop_monitor {
            monitor.stop();
        }
        let _ = default_gui_platform().cleanup_input_state();
        if !self.hidden_pids.is_empty() {
            let _ = default_gui_platform().unhide_apps(&self.hidden_pids);
        }
        if let Some(lock) = &self.lock {
            lock.release(&self.conversation_id, self.process_id);
        }
    }
}

pub(super) fn begin_gui_action_session(
    invocation: &ToolInvocation,
    tool_name: &'static str,
    acquire_lock: bool,
) -> Result<GuiActionSession, FunctionCallError> {
    let conversation_id = invocation.session.conversation_id.to_string();
    let process_id = std::process::id();
    let lock = if acquire_lock {
        Some(FileGuiPhysicalResourceLock::acquire(
            gui_lock_path(invocation),
            &conversation_id,
            process_id,
            tool_name,
        )?)
    } else {
        None
    };
    let emergency_stop_monitor = if acquire_lock {
        match default_gui_platform().start_emergency_stop_monitor() {
            Ok(monitor) => monitor,
            Err(error) => {
                if let Some(lock) = &lock {
                    lock.release(&conversation_id, process_id);
                }
                return Err(error);
            }
        }
    } else {
        None
    };
    Ok(GuiActionSession {
        conversation_id,
        process_id,
        lock,
        emergency_stop_monitor,
        hidden_pids: Vec::new(),
    })
}

fn gui_lock_path(invocation: &ToolInvocation) -> PathBuf {
    std::env::var_os("CODEX_GUI_LOCK_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            invocation
                .turn
                .config
                .codex_home
                .join("gui")
                .join("physical-resource.lock")
        })
}

fn read_lock_holder(path: &PathBuf) -> Option<GuiPhysicalResourceLockHolder> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn format_lock_holder(holder: Option<&GuiPhysicalResourceLockHolder>) -> String {
    let Some(holder) = holder else {
        return "another GUI session".to_string();
    };
    let mut parts = Vec::new();
    if let Some(tool_name) = holder.tool_name.as_deref() {
        parts.push(format!("tool {tool_name}"));
    }
    if let Some(pid) = holder.pid {
        parts.push(format!("pid {pid}"));
    }
    if let Some(acquired_at) = holder.acquired_at {
        parts.push(format!("acquired {acquired_at}"));
    }
    if parts.is_empty() {
        "another GUI session".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(unix)]
fn is_process_alive(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn is_process_alive(pid: Option<u32>) -> bool {
    pid.is_some()
}

fn unix_timestamp_ms() -> u64 {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_lock_holder_matches_user_facing_shape() {
        let holder = GuiPhysicalResourceLockHolder {
            session_id: Some("other-thread".to_string()),
            pid: Some(4242),
            acquired_at: None,
            tool_name: Some("gui_click".to_string()),
        };

        assert_eq!(
            format_lock_holder(Some(&holder)),
            "tool gui_click, pid 4242"
        );
    }
}
