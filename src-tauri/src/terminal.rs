use std::process::Command;

pub fn focus_terminal_for_pid(pid: u32) -> Result<(), String> {
    // First, get the TTY for this process
    let tty = get_tty_for_pid(pid)?;

    // Try tmux first (if the process is running inside tmux)
    if focus_tmux_pane_by_tty(&tty).is_ok() {
        return Ok(());
    }

    // Try iTerm2 next
    if focus_iterm_by_tty(&tty).is_ok() {
        return Ok(());
    }

    // Fall back to Terminal.app
    focus_terminal_app_by_tty(&tty)
}

/// Get the TTY device for a given PID using ps command
fn get_tty_for_pid(pid: u32) -> Result<String, String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "tty="])
        .output()
        .map_err(|e| format!("Failed to get TTY: {}", e))?;

    if output.status.success() {
        let tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tty.is_empty() || tty == "??" {
            Err("Process has no TTY".to_string())
        } else {
            Ok(tty)
        }
    } else {
        Err("Failed to get TTY for process".to_string())
    }
}

/// Focus a tmux pane by matching its TTY
/// Returns Ok if the pane was found and focused, Err otherwise
fn focus_tmux_pane_by_tty(tty: &str) -> Result<(), String> {
    // Check if tmux is running by listing panes
    let output = Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_tty} #{session_name}:#{window_index}.#{pane_index}"])
        .output()
        .map_err(|e| format!("Failed to run tmux: {}", e))?;

    if !output.status.success() {
        return Err("tmux not running or no sessions".to_string());
    }

    let panes = String::from_utf8_lossy(&output.stdout);

    // Find the pane with matching TTY
    // TTY from ps is like "ttys003", tmux returns "/dev/ttys003"
    for line in panes.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let pane_tty = parts[0];
            let target = parts[1];

            // Match TTY (handle both with and without /dev/ prefix)
            if pane_tty.contains(tty) || pane_tty.ends_with(tty) {
                // Select the window and pane in tmux
                let _ = Command::new("tmux")
                    .args(["select-window", "-t", target])
                    .output();

                let _ = Command::new("tmux")
                    .args(["select-pane", "-t", target])
                    .output();

                // Now we need to focus the terminal app that's running tmux
                // Try to find and focus it
                focus_tmux_client_terminal()?;

                return Ok(());
            }
        }
    }

    Err("Pane not found in tmux".to_string())
}

/// Focus the terminal application that is running the tmux client
fn focus_tmux_client_terminal() -> Result<(), String> {
    // Get the tmux client TTY
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{client_tty}"])
        .output()
        .map_err(|e| format!("Failed to get tmux client tty: {}", e))?;

    if !output.status.success() {
        // No active client, try to activate any terminal with tmux
        return focus_any_terminal_with_tmux();
    }

    let client_tty = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if client_tty.is_empty() {
        return focus_any_terminal_with_tmux();
    }

    // Extract just the tty name (e.g., "ttys003" from "/dev/ttys003")
    let tty_name = client_tty.split('/').last().unwrap_or(&client_tty);

    // Try to focus the terminal running this TTY
    if focus_iterm_by_tty(tty_name).is_ok() {
        return Ok(());
    }

    if focus_terminal_app_by_tty(tty_name).is_ok() {
        return Ok(());
    }

    // Last resort: just activate any terminal that might be running tmux
    focus_any_terminal_with_tmux()
}

/// Fallback: Focus any terminal app that might be running tmux
fn focus_any_terminal_with_tmux() -> Result<(), String> {
    // Try iTerm2 first, then Terminal.app
    let script = r#"
        tell application "System Events"
            if exists process "iTerm2" then
                tell application "iTerm2" to activate
                return "found"
            else if exists process "Terminal" then
                tell application "Terminal" to activate
                return "found"
            end if
        end tell
        return "not found"
    "#;

    execute_applescript(script)
}

fn focus_iterm_by_tty(tty: &str) -> Result<(), String> {
    let script = format!(r#"
        tell application "System Events"
            if not (exists process "iTerm2") then
                error "iTerm2 not running"
            end if
        end tell

        tell application "iTerm2"
            activate
            repeat with w in windows
                repeat with t in tabs of w
                    repeat with s in sessions of t
                        if tty of s contains "{}" then
                            select s
                            select t
                            set index of w to 1
                            return "found"
                        end if
                    end repeat
                end repeat
            end repeat
        end tell
        return "not found"
    "#, tty);

    execute_applescript(&script)
}

fn focus_terminal_app_by_tty(tty: &str) -> Result<(), String> {
    // Check if Terminal is running first
    let check_script = r#"
        tell application "System Events"
            return exists process "Terminal"
        end tell
    "#;

    let check_output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(check_script)
        .output()
        .map_err(|e| format!("Failed to check Terminal: {}", e))?;

    let is_running = String::from_utf8_lossy(&check_output.stdout).trim() == "true";
    if !is_running {
        return Err("Terminal is not running".to_string());
    }

    let script = format!(r#"
        tell application "Terminal"
            activate
            repeat with w in windows
                repeat with t in tabs of w
                    try
                        if tty of t contains "{}" then
                            set selected of t to true
                            set index of w to 1
                            return "found"
                        end if
                    end try
                end repeat
            end repeat
        end tell
        return "not found"
    "#, tty);

    execute_applescript(&script)
}

fn execute_applescript(script: &str) -> Result<(), String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to execute AppleScript: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Check if script returned "found" - otherwise consider it a failure
        if stdout == "not found" {
            Err("Tab not found".to_string())
        } else {
            Ok(())
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("AppleScript error: {}", stderr))
    }
}

pub fn focus_terminal_by_path(path: &str) -> Result<(), String> {
    // Fallback: focus by matching session name (which often contains the path) in iTerm2
    let script = format!(r#"
        tell application "System Events"
            if exists process "iTerm2" then
                tell application "iTerm2"
                    activate
                    repeat with w in windows
                        repeat with t in tabs of w
                            repeat with s in sessions of t
                                if name of s contains "{}" then
                                    select s
                                    select t
                                    set index of w to 1
                                    return "found"
                                end if
                            end repeat
                        end repeat
                    end repeat
                end tell
            end if
        end tell
        return "not found"
    "#, path.split('/').last().unwrap_or(path));

    execute_applescript(&script)
}
