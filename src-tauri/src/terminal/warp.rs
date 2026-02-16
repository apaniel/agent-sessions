use super::applescript::execute_applescript;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;

/// Focus Warp and switch to the tab containing the given TTY.
///
/// Strategy: write a unique marker to the target TTY via an OSC title escape
/// sequence. This changes the tab title to our marker. Then cycle through Warp
/// tabs with Cmd+Shift+] looking for the marker. After finding it, reset the
/// tab title to the project folder name.
pub fn focus_warp(tty: &str, project_path: &str) -> Result<(), String> {
    let tty_path = if tty.starts_with("/dev/") {
        tty.to_string()
    } else {
        format!("/dev/{}", tty)
    };
    let folder = project_path.split('/').last().unwrap_or(project_path);
    let marker = format!(
        "__FOCUS_{}__",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    // Step 1: Write marker title to the target TTY
    if let Err(_e) = set_tty_title(&tty_path, &marker) {
        // Fall back to just activating Warp
        let script = r#"tell application "Warp" to activate
            return "found""#;
        return execute_applescript(script);
    }

    // Give Warp a moment to process the OSC title change
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Step 2: Activate Warp and cycle tabs to find the marker
    // Note: we cycle a fixed 30 times instead of detecting cycle-completion by
    // title, because duplicate tab titles cause false cycle-detection.
    let script = format!(
        r#"
        tell application "Warp" to activate
        delay 0.2
        tell application "System Events"
            tell process "Warp"
                if name of window 1 contains "{marker}" then return "found"
                repeat 30 times
                    key code 30 using {{command down, shift down}}
                    delay 0.1
                    if name of window 1 contains "{marker}" then return "found"
                end repeat
            end tell
        end tell
        return "not found"
    "#,
        marker = marker
    );

    let result = execute_applescript(&script);

    // Step 3: Restore the tab title to the folder name
    let _ = set_tty_title(&tty_path, folder);

    result
}

/// Check if Warp owns this TTY via lsof, then activate Warp.
/// Used in the fallback path when we don't know which terminal it is.
pub fn focus_warp_by_tty(tty: &str) -> Result<(), String> {
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

    if !stdout.lines().any(|line| line.contains("Warp")) {
        return Err("TTY not owned by Warp".to_string());
    }

    let script = r#"
        tell application "Warp" to activate
        return "found"
    "#;
    execute_applescript(script)
}

/// Write an OSC escape sequence to set the tab title on a TTY device.
fn set_tty_title(tty_path: &str, title: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(tty_path)
        .map_err(|e| format!("Failed to open {}: {}", tty_path, e))?;
    // OSC 0 = set window/icon title
    write!(file, "\x1b]0;{}\x07", title)
        .map_err(|e| format!("Failed to write to {}: {}", tty_path, e))
}
