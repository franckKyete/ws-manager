use std::process::Command;

pub fn send_dbus_notification(summary: &str, body: &str, icon: &str, timeout_ms: i32) -> bool {
    // 1. Try gdbus
    let gdbus_args = [
        "call",
        "--session",
        "--dest",
        "org.freedesktop.Notifications",
        "--object-path",
        "/org/freedesktop/Notifications",
        "--method",
        "org.freedesktop.Notifications.Notify",
        "ws",
        "0",
        icon,
        summary,
        body,
        "[]",
        "{}",
        &timeout_ms.to_string(),
    ];

    if let Ok(out) = Command::new("gdbus").args(gdbus_args).output() {
        if out.status.success() {
            return true;
        }
    }

    // 2. Fallback to notify-send
    let notify_args = [
        "-a",
        "ws",
        "-i",
        icon,
        "-t",
        &timeout_ms.to_string(),
        summary,
        body,
    ];

    if let Ok(out) = Command::new("notify-send").args(notify_args).output() {
        if out.status.success() {
            return true;
        }
    }

    false
}

pub fn notify_auto_save_success(workspace_name: &str, project: Option<&str>) -> bool {
    let summary = "wshub Auto-Save";
    let body = match project {
        Some(p) => format!("Workspace @{} successfully saved to {}", workspace_name, p),
        None => format!("Workspace @{} successfully saved", workspace_name),
    };
    send_dbus_notification(summary, &body, "document-save", 5000)
}

pub fn notify_auto_save_failure(workspace_name: &str, error: &str, project: Option<&str>) -> bool {
    let summary = "wshub Auto-Save Failed";
    let mut clean_err = error.trim().to_string();
    if clean_err.len() > 200 {
        clean_err = format!("{}...", &clean_err[..197]);
    }
    let body = match project {
        Some(p) => format!("Failed to save @{} to {}: {}", workspace_name, p, clean_err),
        None => format!("Failed to save @{}: {}", workspace_name, clean_err),
    };
    send_dbus_notification(summary, &body, "dialog-error", 8000)
}

pub fn notify_blueprint_push_success(project: &str, revision: Option<&str>) -> bool {
    let summary = "wshub Blueprint Updated";
    let rev_text = revision.map(|r| format!(" (v{})", r)).unwrap_or_default();
    let body = format!("Project blueprint for {project} updated{rev_text}");
    send_dbus_notification(summary, &body, "document-save", 5000)
}

pub fn notify_blueprint_push_failure(project: &str, error: &str) -> bool {
    let summary = "wshub Blueprint Push Failed";
    let mut clean_err = error.trim().to_string();
    if clean_err.len() > 200 {
        clean_err = format!("{}...", &clean_err[..197]);
    }
    let body = format!("Failed to push blueprint for {project}: {clean_err}");
    send_dbus_notification(summary, &body, "dialog-error", 8000)
}
