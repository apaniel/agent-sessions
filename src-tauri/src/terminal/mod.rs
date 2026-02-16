mod applescript;
mod iterm;
mod terminal_app;
mod tmux;
pub mod vscode;
mod warp;

use applescript::execute_applescript;

/// Focus the terminal containing the Claude process with the given PID
pub fn focus_terminal_for_pid(pid: u32, hint: &str, project_path: &str) -> Result<(), String> {
    // First, get the TTY for this process
    let tty = get_tty_for_pid(pid)?;

    // If we know which terminal app it is, go directly there
    match hint {
        "cursor" | "vscode" => return vscode::focus_vscode_by_tty(&tty, project_path),
        "warp" => return warp::focus_warp(&tty, project_path),
        "iterm2" => return iterm::focus_iterm_by_tty(&tty),
        "terminal" => return terminal_app::focus_terminal_app_by_tty(&tty),
        "tmux" => {
            if tmux::focus_tmux_pane_by_tty(&tty).is_ok() {
                return Ok(());
            }
        }
        _ => {}
    }

    // Fallback: try all terminals in order
    if tmux::focus_tmux_pane_by_tty(&tty).is_ok() {
        return Ok(());
    }
    if iterm::focus_iterm_by_tty(&tty).is_ok() {
        return Ok(());
    }
    if warp::focus_warp_by_tty(&tty).is_ok() {
        return Ok(());
    }
    if vscode::focus_vscode_by_tty(&tty, project_path).is_ok() {
        return Ok(());
    }
    terminal_app::focus_terminal_app_by_tty(&tty)
}

/// Fallback: focus terminal by matching path in session name
pub fn focus_terminal_by_path(path: &str) -> Result<(), String> {
    // Fallback: focus by matching session name (which often contains the path) in iTerm2
    let folder = path.split('/').last().unwrap_or(path);
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
    "#, folder);

    // Try iTerm2 first
    if execute_applescript(&script).is_ok() {
        return Ok(());
    }

    // Try Warp — no per-tab matching, but we can activate it
    let warp_script = r#"
        tell application "System Events"
            if exists process "Warp" then
                tell application "Warp" to activate
                return "found"
            end if
        end tell
        return "not found"
    "#;

    execute_applescript(warp_script)
}

use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::Lazy;

/// Cache terminal detection results per PID (terminal doesn't change for a running process)
static TERMINAL_CACHE: Lazy<Mutex<HashMap<u32, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Detect which terminal application owns the given PID's TTY (cached)
pub fn detect_terminal_for_pid(pid: u32) -> String {
    // Check cache first
    if let Ok(cache) = TERMINAL_CACHE.lock() {
        if let Some(cached) = cache.get(&pid) {
            return cached.clone();
        }
    }

    let result = detect_terminal_for_pid_uncached(pid);

    // Cache the result
    if let Ok(mut cache) = TERMINAL_CACHE.lock() {
        cache.insert(pid, result.clone());
    }

    result
}

fn detect_terminal_for_pid_uncached(pid: u32) -> String {
    let tty = match get_tty_for_pid(pid) {
        Ok(t) => t,
        Err(_) => return "unknown".to_string(),
    };

    let tty_path = if tty.starts_with("/dev/") {
        tty.clone()
    } else {
        format!("/dev/{}", tty)
    };

    // Check if running inside tmux
    if let Ok(output) = std::process::Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_tty}"])
        .output()
    {
        if output.status.success() {
            let panes = String::from_utf8_lossy(&output.stdout);
            if panes.lines().any(|line| line.contains(&tty)) {
                return "tmux".to_string();
            }
        }
    }

    // Use lsof to find which app owns the TTY
    if let Ok(output) = std::process::Command::new("lsof").arg(&tty_path).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("Cursor") {
                return "cursor".to_string();
            }
            if line.contains("Code") {
                return "vscode".to_string();
            }
            if line.contains("Warp") {
                return "warp".to_string();
            }
            if line.contains("iTerm2") {
                return "iterm2".to_string();
            }
            if line.contains("Terminal") {
                return "terminal".to_string();
            }
        }
    }

    // Fallback: walk up the process tree to find the terminal app
    // Some terminals (e.g. Warp) don't show up in lsof for the TTY
    detect_terminal_from_parent(pid)
}

/// Get the TTY device for a given PID using ps command
fn get_tty_for_pid(pid: u32) -> Result<String, String> {
    use std::process::Command;

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

/// Walk up the process tree to find a known terminal application
fn detect_terminal_from_parent(pid: u32) -> String {
    let mut current_pid = pid;

    // Walk up to 10 levels to avoid infinite loops
    for _ in 0..10 {
        let output = match std::process::Command::new("ps")
            .args(["-p", &current_pid.to_string(), "-o", "ppid=,comm="])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => break,
        };

        let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let mut parts = line.splitn(2, |c: char| c.is_whitespace());
        let ppid_str = match parts.next() {
            Some(s) => s.trim(),
            None => break,
        };
        let comm = parts.next().unwrap_or("").trim();

        if comm.contains("Warp") {
            return "warp".to_string();
        }
        if comm.contains("Cursor") {
            return "cursor".to_string();
        }
        if comm.contains("Code") || comm.contains("code") {
            return "vscode".to_string();
        }
        if comm.contains("iTerm") {
            return "iterm2".to_string();
        }
        if comm.ends_with("Terminal") {
            return "terminal".to_string();
        }

        match ppid_str.parse::<u32>() {
            Ok(ppid) if ppid > 1 => current_pid = ppid,
            _ => break, // Reached init/launchd or invalid
        }
    }

    "unknown".to_string()
}
