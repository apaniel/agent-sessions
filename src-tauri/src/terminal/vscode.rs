use std::process::Command;
use super::applescript::execute_applescript;

/// Focus VS Code or Cursor by detecting if the TTY belongs to them via lsof
pub fn focus_vscode_by_tty(tty: &str, project_path: &str) -> Result<(), String> {
    let tty_path = if tty.starts_with("/dev/") {
        tty.to_string()
    } else {
        format!("/dev/{}", tty)
    };

    let output = Command::new("lsof")
        .arg(&tty_path)
        .output()
        .map_err(|e| format!("Failed to run lsof: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Detect which editor owns this TTY
    for line in stdout.lines() {
        if line.contains("Cursor") {
            return activate_app_window("Cursor", project_path);
        }
        if line.contains("Code") {
            return activate_app_window("Visual Studio Code", project_path);
        }
    }

    Err("TTY not owned by VS Code or Cursor".to_string())
}

/// Activate the app and raise the window matching the project folder name.
/// Does NOT use `tell application to activate` which brings ALL windows to front.
/// Instead, raises only the specific project window via AXRaise.
pub fn activate_app_window(app_name: &str, project_path: &str) -> Result<(), String> {
    let folder = project_path.split('/').last().unwrap_or(project_path);

    // Use System Events to find and raise ONLY the matching window.
    // The layout_session_windows function handles full activation later
    // with NSApplicationActivateIgnoringOtherApps (without AllWindows).
    let script = format!(
        r#"
        tell application "System Events"
            tell process "{app_name}"
                repeat with w in windows
                    if name of w contains "{folder}" then
                        perform action "AXRaise" of w
                        return "found"
                    end if
                end repeat
            end tell
        end tell
        return "not-found"
    "#,
        app_name = app_name,
        folder = folder
    );
    execute_applescript(&script)
}
