use std::process::Command;

pub fn focus_terminal_for_pid(pid: u32) -> Result<(), String> {
    // Try iTerm2 first, then Terminal.app
    if focus_iterm(pid).is_ok() {
        return Ok(());
    }

    focus_terminal_app(pid)
}

fn focus_iterm(pid: u32) -> Result<(), String> {
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
                            select t
                            return
                        end if
                    end repeat
                end repeat
            end repeat
        end tell
    "#, pid);

    execute_applescript(&script)
}

fn focus_terminal_app(pid: u32) -> Result<(), String> {
    let script = format!(r#"
        tell application "Terminal"
            activate
            set targetFound to false
            repeat with w in windows
                repeat with t in tabs of w
                    try
                        set tabProcesses to processes of t
                        repeat with p in tabProcesses
                            if p contains "{}" then
                                set selected of t to true
                                set index of w to 1
                                set targetFound to true
                                exit repeat
                            end if
                        end repeat
                    end try
                    if targetFound then exit repeat
                end repeat
                if targetFound then exit repeat
            end repeat
        end tell
    "#, pid);

    execute_applescript(&script)
}

fn execute_applescript(script: &str) -> Result<(), String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to execute AppleScript: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("AppleScript error: {}", stderr))
    }
}

pub fn focus_terminal_by_path(path: &str) -> Result<(), String> {
    // Fallback: focus by working directory path
    let script = format!(r#"
        tell application "Terminal"
            activate
            repeat with w in windows
                repeat with t in tabs of w
                    try
                        if (do script "pwd" in t) contains "{}" then
                            set selected of t to true
                            set index of w to 1
                            return
                        end if
                    end try
                end repeat
            end repeat
        end tell
    "#, path);

    execute_applescript(&script)
}
