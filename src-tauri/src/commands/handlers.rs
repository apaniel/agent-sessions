use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use crate::session::{get_sessions, convert_path_to_dir_name, SessionsResponse};
use crate::terminal;

// Store current shortcut for unregistration
static CURRENT_SHORTCUT: Mutex<Option<Shortcut>> = Mutex::new(None);

// Track last opened URL per project path to avoid duplicate tabs
static CHROME_URLS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Path to the persisted Chrome window ID mapping.
fn chrome_windows_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap()
        .join(".agent-sessions")
        .join("chrome-windows.json")
}

/// Load Chrome window IDs from disk (survives app restarts).
fn load_chrome_window_ids() -> HashMap<String, i64> {
    match std::fs::read_to_string(chrome_windows_path()) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save Chrome window IDs to disk.
fn save_chrome_window_ids(map: &HashMap<String, i64>) {
    if let Ok(json) = serde_json::to_string_pretty(map) {
        let _ = std::fs::write(chrome_windows_path(), json);
    }
}

/// Path to the persisted Cursor project tracking file.
fn cursor_projects_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap()
        .join(".agent-sessions")
        .join("cursor-projects.json")
}

/// Load tracked Cursor project paths from disk.
fn load_cursor_projects() -> HashSet<String> {
    match std::fs::read_to_string(cursor_projects_path()) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HashSet::new(),
    }
}

/// Save tracked Cursor project paths to disk.
fn save_cursor_projects(set: &HashSet<String>) {
    if let Ok(json) = serde_json::to_string_pretty(set) {
        let _ = std::fs::write(cursor_projects_path(), json);
    }
}

/// Check if a Cursor window exists for the given project path by looking for a window
/// whose title contains the folder name.
fn cursor_window_exists_for_project(project_path: &str) -> bool {
    let folder = project_path.split('/').last().unwrap_or(project_path);
    let script = format!(
        r#"tell application "System Events"
            if not (exists process "Cursor") then return "not-found"
            tell process "Cursor"
                repeat with w in windows
                    if name of w contains "{}" then return "found"
                end repeat
            end tell
            return "not-found"
        end tell"#,
        folder
    );
    match run_applescript(&script) {
        Ok(r) => r == "found",
        Err(_) => false,
    }
}

/// Read the configured Chrome profile from ~/.agent-sessions/config.json.
/// Returns Some("Profile 1") etc. if configured, None to use isolated mode.
fn read_chrome_profile() -> Option<String> {
    let home = dirs::home_dir()?;
    let config_path = home.join(".agent-sessions").join("config.json");
    let content = std::fs::read_to_string(config_path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&content).ok()?;
    config.get("chrome_profile")?.as_str().map(|s| s.to_string())
}

/// Find the main Chrome browser PID (not an agent-sessions isolated instance).
fn find_main_chrome_pid() -> Option<u32> {
    let output = std::process::Command::new("ps")
        .arg("-ww")
        .arg("-eo")
        .arg("pid,args")
        .output()
        .ok()?;

    let home = dirs::home_dir()?;
    let agent_sessions_needle = format!(
        "--user-data-dir={}",
        home.join(".agent-sessions").join("chrome-profiles").display()
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.contains("Google Chrome")
            && !line.contains("--type=")
            && !line.contains(&agent_sessions_needle)
        {
            let pid_str = line.split_whitespace().next()?;
            return pid_str.parse().ok();
        }
    }
    None
}

/// Create a new Chrome window via AppleScript and return its ID.
/// This is synchronous and reliable — no polling needed.
fn create_chrome_window(url: Option<&str>) -> Result<i64, String> {
    let script = match url {
        Some(u) => {
            let escaped = u.replace('\\', "\\\\").replace('"', "\\\"");
            format!(
                r#"tell application "Google Chrome"
                    make new window
                    set URL of active tab of window 1 to "{}"
                    return id of window 1
                end tell"#,
                escaped
            )
        }
        None => r#"tell application "Google Chrome"
            make new window
            return id of window 1
        end tell"#.to_string(),
    };
    let result = run_applescript(&script)?;
    result.trim().parse::<i64>()
        .map_err(|e| format!("Failed to parse window ID: {}", e))
}

/// Check if a Chrome window still exists by ID.
/// Uses text comparison to avoid AppleScript type-mismatch issues with Chrome's window IDs.
fn chrome_window_exists(window_id: i64) -> bool {
    let script = format!(
        r#"tell application "Google Chrome"
            repeat with w in windows
                if (id of w as text) is "{}" then return "found"
            end repeat
            return "not-found"
        end tell"#,
        window_id
    );
    match run_applescript(&script) {
        Ok(r) => {
            let exists = r == "found";
            log::info!("chrome_window_exists({}): exists={}", window_id, exists);
            exists
        }
        Err(e) => {
            log::warn!("chrome_window_exists({}): AppleScript error: {}", window_id, e);
            true
        }
    }
}

/// Raise a specific Chrome window by ID.
/// Uses Chrome AppleScript to reorder the window to index 1, then activates Chrome
/// via NSRunningApplication with main-window-only flag so only that window comes forward.
fn raise_chrome_window(window_id: i64) {
    // Tell Chrome to make our window the frontmost (index 1 = key window)
    let _ = run_applescript(&format!(
        r#"tell application "Google Chrome"
            repeat with w in windows
                if (id of w as text) is "{}" then
                    set index of w to 1
                    exit repeat
                end if
            end repeat
        end tell"#,
        window_id
    ));

    // Activate Chrome bringing only its key/main window to front (not all windows)
    if let Some(chrome_pid) = find_main_chrome_pid() {
        focus_pid_main_window_only(chrome_pid);
    }
}

/// Get all active Claude Code sessions
#[tauri::command]
pub fn get_all_sessions() -> SessionsResponse {
    get_sessions()
}

/// Focus the terminal containing a specific session and auto-layout windows
#[tauri::command]
pub fn focus_session(pid: u32, project_path: String, terminal_app: String) -> Result<(), String> {
    terminal::focus_terminal_for_pid(pid, &terminal_app, &project_path)
        .or_else(|_| terminal::focus_terminal_by_path(&project_path))?;

    // Focus companion Cursor if tracked and still open (best-effort)
    if load_cursor_projects().contains(&project_path) {
        if cursor_window_exists_for_project(&project_path) {
            let _ = terminal::vscode::activate_app_window("Cursor", &project_path);
        } else {
            // Window was manually closed — clean up tracking
            let mut projects = load_cursor_projects();
            projects.remove(&project_path);
            save_cursor_projects(&projects);
        }
    }

    // Layout windows after focusing (best-effort, don't fail the command)
    let _ = layout_session_windows(pid, &terminal_app, &project_path);

    Ok(())
}

/// Update the tray icon title with session counts
#[tauri::command]
pub fn update_tray_title(app: tauri::AppHandle, total: usize, waiting: usize) -> Result<(), String> {
    let title = if waiting > 0 {
        format!("{} ({} waiting)", total, waiting)
    } else if total > 0 {
        format!("{}", total)
    } else {
        String::new()
    };

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_title(Some(&title))
            .map_err(|e| format!("Failed to set tray title: {}", e))?;
    }
    Ok(())
}

/// Register a global keyboard shortcut to toggle the window
#[tauri::command]
pub fn register_shortcut(app: tauri::AppHandle, shortcut: String) -> Result<(), String> {
    // Unregister any existing shortcut first
    if let Some(old_shortcut) = CURRENT_SHORTCUT.lock().unwrap().take() {
        let _ = app.global_shortcut().unregister(old_shortcut);
    }

    // Parse the shortcut string
    let parsed_shortcut: Shortcut = shortcut.parse()
        .map_err(|e| format!("Invalid shortcut format: {}", e))?;

    // Register the new shortcut - toggle window visibility
    app.global_shortcut()
        .on_shortcut(parsed_shortcut.clone(), move |app, _shortcut, event| {
            // Only handle key press, not release
            if event.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
                return;
            }

            if let Some(window) = app.get_webview_window("main") {
                let is_visible = window.is_visible().unwrap_or(false);
                let is_focused = window.is_focused().unwrap_or(false);

                // If window is visible AND focused, hide it
                // Otherwise, show and focus it
                if is_visible && is_focused {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .map_err(|e| format!("Failed to register shortcut: {}", e))?;

    // Store the shortcut for later unregistration
    *CURRENT_SHORTCUT.lock().unwrap() = Some(parsed_shortcut);

    Ok(())
}

/// Unregister the current global keyboard shortcut
#[tauri::command]
pub fn unregister_shortcut(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(shortcut) = CURRENT_SHORTCUT.lock().unwrap().take() {
        app.global_shortcut()
            .unregister(shortcut)
            .map_err(|e| format!("Failed to unregister shortcut: {}", e))?;
    }
    Ok(())
}

/// Find the main Chrome browser process PID for a given --user-data-dir.
fn chrome_pid_for_profile(profile_dir: &std::path::Path) -> Option<u32> {
    // -ww ensures full command line output (no truncation)
    let output = std::process::Command::new("ps")
        .arg("-ww")
        .arg("-eo")
        .arg("pid,args")
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let needle = format!("--user-data-dir={}", profile_dir.display());

    for line in stdout.lines() {
        let line = line.trim();
        // Match the main browser process (no --type= flag — helpers have --type=renderer etc.)
        if line.contains("Google Chrome") && line.contains(&needle) && !line.contains("--type=") {
            let pid_str = line.split_whitespace().next()?;
            return pid_str.parse().ok();
        }
    }
    None
}

// Accessibility API FFI — used to unminimize windows by PID.
// AXWindows includes minimized windows (unlike System Events).
mod ax {
    use std::ffi::c_void;

    pub type AXUIElementRef = *const c_void;
    pub type CFTypeRef = *const c_void;
    pub type CFStringRef = *const c_void;
    pub type CFArrayRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        pub fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        pub fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRelease(cf: CFTypeRef);
        pub fn CFArrayGetCount(array: CFArrayRef) -> isize;
        pub fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> CFTypeRef;
        pub fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
        pub fn CFStringCreateWithCString(
            alloc: CFTypeRef,
            c_str: *const i8,
            encoding: u32,
        ) -> CFStringRef;
        pub static kCFBooleanFalse: CFTypeRef;
        pub static kCFBooleanTrue: CFTypeRef;
    }

    const KCF_STRING_ENCODING_UTF8: u32 = 0x08000100;

    pub fn cfstr(s: &str) -> CFStringRef {
        let c = std::ffi::CString::new(s).unwrap();
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8) }
    }
}

/// Bring a specific process to the foreground and unminimize its windows.
fn activate_pid(pid: u32) {
    unminimize_pid(pid);
    focus_pid(pid);
}

/// Unminimize all windows for a process via Accessibility API.
fn unminimize_pid(pid: u32) {
    unsafe {
        let app_ref = ax::AXUIElementCreateApplication(pid as i32);
        if !app_ref.is_null() {
            let attr_windows = ax::cfstr("AXWindows");
            let attr_minimized = ax::cfstr("AXMinimized");
            let mut windows: ax::CFTypeRef = std::ptr::null();

            if ax::AXUIElementCopyAttributeValue(app_ref, attr_windows, &mut windows) == 0
                && !windows.is_null()
            {
                let count = ax::CFArrayGetCount(windows);
                for i in 0..count {
                    let win = ax::CFArrayGetValueAtIndex(windows, i);
                    let mut val: ax::CFTypeRef = std::ptr::null();
                    if ax::AXUIElementCopyAttributeValue(win, attr_minimized, &mut val) == 0
                        && !val.is_null()
                    {
                        if ax::CFBooleanGetValue(val) {
                            ax::AXUIElementSetAttributeValue(
                                win,
                                attr_minimized,
                                ax::kCFBooleanFalse,
                            );
                        }
                        ax::CFRelease(val);
                    }
                }
                ax::CFRelease(windows);
            }

            ax::CFRelease(attr_windows);
            ax::CFRelease(attr_minimized);
            ax::CFRelease(app_ref);
        }
    }
}

/// Bring a process to the foreground without unminimizing windows.
/// Brings ALL windows of the process to the front.
fn focus_pid(pid: u32) {
    focus_pid_with_options(pid, 3); // AllWindows | IgnoringOtherApps
}

/// Bring a process to the foreground, but only its key/main window — not ALL windows.
/// Other windows of the process remain in their current Z-order.
fn focus_pid_main_window_only(pid: u32) {
    focus_pid_with_options(pid, 2); // IgnoringOtherApps only (no AllWindows)
}

fn focus_pid_with_options(pid: u32, options: usize) {
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let cls = class!(NSRunningApplication);
        let app: *mut objc::runtime::Object =
            msg_send![cls, runningApplicationWithProcessIdentifier: pid as i32];
        if !app.is_null() {
            let _: bool = msg_send![app, activateWithOptions: options];
        }
    }
}

/// Minimize all windows for a specific process via Accessibility API.
fn minimize_pid(pid: u32) {
    unsafe {
        let app_ref = ax::AXUIElementCreateApplication(pid as i32);
        if app_ref.is_null() {
            return;
        }

        let attr_windows = ax::cfstr("AXWindows");
        let attr_minimized = ax::cfstr("AXMinimized");
        let mut windows: ax::CFTypeRef = std::ptr::null();

        if ax::AXUIElementCopyAttributeValue(app_ref, attr_windows, &mut windows) == 0
            && !windows.is_null()
        {
            let count = ax::CFArrayGetCount(windows);
            for i in 0..count {
                let win = ax::CFArrayGetValueAtIndex(windows, i);
                ax::AXUIElementSetAttributeValue(win, attr_minimized, ax::kCFBooleanTrue);
            }
            ax::CFRelease(windows);
        }

        ax::CFRelease(attr_windows);
        ax::CFRelease(attr_minimized);
        ax::CFRelease(app_ref);
    }
}

/// Minimize companion windows/instances belonging to other sessions (not the current one).
/// Handles both Chrome and Cursor companions.
fn minimize_other_companion_instances(current_project_path: &str) {
    // --- Chrome ---
    if read_chrome_profile().is_some() {
        // Real profile mode: all sessions share one Chrome process.
        // Minimize specific windows that belong to other tracked sessions.
        let windows = load_chrome_window_ids();
        let ids_to_minimize: Vec<i64> = windows.iter()
            .filter(|(path, _)| path.as_str() != current_project_path)
            .map(|(_, &id)| id)
            .collect();

        if !ids_to_minimize.is_empty() {
            let conditions: Vec<String> = ids_to_minimize.iter()
                .map(|id| format!("(id of w as text) is \"{}\"", id))
                .collect();

            let script = format!(
                r#"tell application "Google Chrome"
                    repeat with w in windows
                        if {} then
                            set miniaturized of w to true
                        end if
                    end repeat
                end tell"#,
                conditions.join(" or ")
            );
            let _ = run_applescript(&script);
        }
    } else {
        // Isolated mode: each session has its own Chrome process.
        if let Ok(current_profile) = chrome_profile_dir(current_project_path) {
            if let Some(home) = dirs::home_dir() {
                let base_needle = format!(
                    "--user-data-dir={}",
                    home.join(".agent-sessions").join("chrome-profiles").display()
                );
                let current_needle = format!("--user-data-dir={}", current_profile.display());

                if let Ok(output) = std::process::Command::new("ps")
                    .arg("-ww")
                    .arg("-eo")
                    .arg("pid,args")
                    .output()
                {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    for line in stdout.lines() {
                        let line = line.trim();
                        if !line.contains("Google Chrome")
                            || !line.contains(&base_needle)
                            || line.contains("--type=")
                        {
                            continue;
                        }
                        if line.contains(&current_needle) {
                            continue;
                        }
                        if let Some(pid) = line.split_whitespace().next().and_then(|s| s.parse::<u32>().ok()) {
                            minimize_pid(pid);
                        }
                    }
                }
            }
        }
    }

    // --- Cursor ---
    // Minimize Cursor windows belonging to other tracked sessions
    let cursor_projects = load_cursor_projects();
    let other_folders: Vec<String> = cursor_projects.iter()
        .filter(|path| path.as_str() != current_project_path)
        .filter_map(|path| path.split('/').last().map(|s| s.to_string()))
        .collect();

    if !other_folders.is_empty() {
        // Build conditions to match windows from other sessions
        let conditions: Vec<String> = other_folders.iter()
            .map(|folder| format!("name of w contains \"{}\"", folder))
            .collect();

        let script = format!(
            r#"tell application "System Events"
                if exists process "Cursor" then
                    tell process "Cursor"
                        repeat with w in windows
                            if {} then
                                set value of attribute "AXMinimized" of w to true
                            end if
                        end repeat
                    end tell
                end if
            end tell"#,
            conditions.join(" or ")
        );
        let _ = run_applescript(&script);
    }
}

/// Get the Chrome profile directory for a given project path.
/// Uses the same path encoding as Claude's project directories for consistency.
fn chrome_profile_dir(project_path: &str) -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir()
        .ok_or_else(|| "Could not determine home directory".to_string())?;
    Ok(home
        .join(".agent-sessions")
        .join("chrome-profiles")
        .join(convert_path_to_dir_name(project_path)))
}

/// Launch a Chrome instance for a session.
/// Chrome instances are linked to project paths (not PIDs), so they persist across session restarts.
/// If chrome_profile is configured in ~/.agent-sessions/config.json, uses the real Chrome profile.
/// Otherwise falls back to isolated per-project profiles.
#[tauri::command]
pub fn launch_chrome(project_name: String, project_path: String, url: Option<String>) -> Result<(), String> {
    use std::process::Command;

    let chrome_binary = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    if !std::path::Path::new(chrome_binary).exists() {
        return Err("Google Chrome not found at /Applications/Google Chrome.app".to_string());
    }

    let key = project_path.clone();
    log::info!("launch_chrome: key={}, url={:?}", key, url);

    // If a Chrome profile is configured, use the real profile
    if let Some(profile) = read_chrome_profile() {
        log::info!("launch_chrome: real profile mode ({})", profile);

        // Load persisted window IDs from disk
        let mut windows = load_chrome_window_ids();
        log::info!("launch_chrome: loaded {} persisted window entries", windows.len());

        if let Some(&window_id) = windows.get(&key) {
            log::info!("launch_chrome: found persisted window_id={} for key", window_id);
            if chrome_window_exists(window_id) {
                log::info!("launch_chrome: window {} still exists, raising it", window_id);
                raise_chrome_window(window_id);
                // Open new URL as tab if needed
                if let Some(ref u) = url {
                    let mut urls = CHROME_URLS.lock().unwrap();
                    if urls.get(&key).map(|lu| lu != u).unwrap_or(true) {
                        let escaped = u.replace('\\', "\\\\").replace('"', "\\\"");
                        let _ = run_applescript(&format!(
                            r#"tell application "Google Chrome"
                                repeat with w in windows
                                    if (id of w as text) is "{}" then
                                        tell w to make new tab with properties {{URL:"{}"}}
                                        return
                                    end if
                                end repeat
                            end tell"#,
                            window_id, escaped
                        ));
                        urls.insert(key, u.clone());
                    }
                }
                return Ok(());
            }
            // Window was closed, remove from persisted map
            log::info!("launch_chrome: window {} no longer exists, removing", window_id);
            windows.remove(&key);
            save_chrome_window_ids(&windows);
        } else {
            log::info!("launch_chrome: no persisted window for this key");
        }

        // Ensure Chrome is running with the right profile before using AppleScript
        // (AppleScript `make new window` doesn't support profile selection)
        if find_main_chrome_pid().is_none() {
            log::info!("launch_chrome: Chrome not running, launching with profile");
            let profile_arg = format!("--profile-directory={}", profile);
            let mut cmd = Command::new(chrome_binary);
            cmd.arg(&profile_arg);
            if let Some(ref u) = url {
                cmd.arg(u);
                CHROME_URLS.lock().unwrap().insert(key.clone(), u.clone());
            }
            cmd.spawn()
                .map_err(|e| format!("Failed to launch Chrome: {}", e))?;

            // Wait for Chrome to start, then capture window ID
            std::thread::sleep(std::time::Duration::from_secs(2));
            let script = r#"tell application "Google Chrome" to return id of window 1"#;
            if let Ok(id_str) = run_applescript(script) {
                if let Ok(id) = id_str.trim().parse::<i64>() {
                    log::info!("launch_chrome: captured initial window_id={}", id);
                    windows.insert(key, id);
                    save_chrome_window_ids(&windows);
                }
            }
            return Ok(());
        }

        // Chrome is running — create window via AppleScript (synchronous, gives us the ID)
        log::info!("launch_chrome: Chrome running, creating new window via AppleScript");
        let window_id = create_chrome_window(url.as_deref())?;
        log::info!("launch_chrome: created window_id={}, persisting", window_id);
        windows.insert(key.clone(), window_id);
        save_chrome_window_ids(&windows);
        if let Some(ref u) = url {
            CHROME_URLS.lock().unwrap().insert(key, u.clone());
        }
        let _ = run_applescript(r#"tell application "Google Chrome" to activate"#);

        return Ok(());
    }

    // --- Isolated mode (no chrome_profile configured) ---
    log::info!("launch_chrome: isolated mode");
    let profile_dir = chrome_profile_dir(&project_path)?;
    log::info!("launch_chrome: profile_dir={}", profile_dir.display());

    std::fs::create_dir_all(&profile_dir)
        .map_err(|e| format!("Failed to create Chrome profile directory: {}", e))?;

    let user_data_arg = format!("--user-data-dir={}", profile_dir.display());

    // Check if Chrome is already running for this profile
    if let Some(chrome_pid) = chrome_pid_for_profile(&profile_dir) {
        log::info!("launch_chrome: found existing Chrome pid={} for profile", chrome_pid);
        let mut urls = CHROME_URLS.lock().unwrap();
        let url_already_open = match (&url, urls.get(&key)) {
            (Some(u), Some(lu)) => u == lu,
            _ => false,
        };

        if let Some(ref u) = url {
            if !url_already_open {
                // Open new URL as tab in existing instance
                let _ = Command::new(chrome_binary)
                    .arg(&user_data_arg)
                    .arg(u)
                    .spawn();
                urls.insert(key, u.clone());
            }
        }

        // Bring existing Chrome to foreground
        activate_pid(chrome_pid);
        return Ok(());
    }

    // Launch a new isolated Chrome instance
    log::info!("launch_chrome: launching new isolated Chrome");
    let mut cmd = Command::new(chrome_binary);
    cmd.arg(&user_data_arg)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--window-name={}", project_name));

    if let Some(ref u) = url {
        cmd.arg(u);
        CHROME_URLS.lock().unwrap().insert(key, u.clone());
    } else {
        // Open a branded start page so the window title shows the project name
        let start_page = format!(
            "data:text/html,<html><head><title>{name}</title></head>\
             <body style=\"font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:%23111;color:%23888\">\
             <h1 style=\"font-weight:300\">{name}</h1></body></html>",
            name = project_name
        );
        cmd.arg(&start_page);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to launch Chrome: {}", e))?;

    Ok(())
}

/// Run an AppleScript and return its stdout.
fn run_applescript(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Run a JXA (JavaScript for Automation) script and return its stdout.
fn run_jxa(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Map terminal app string to macOS process name for AppleScript.
fn terminal_process_name(terminal_app: &str) -> Option<&'static str> {
    match terminal_app {
        "cursor" => Some("Cursor"),
        "vscode" => Some("Code"),
        "warp" => Some("Warp"),
        "iterm2" => Some("iTerm2"),
        "terminal" => Some("Terminal"),
        _ => None, // tmux, unknown — skip positioning
    }
}

/// Get the visible screen bounds (left, top, right, bottom in top-left origin)
/// for the monitor that currently contains the given process's frontmost window.
fn screen_bounds_for_process(process_name: &str) -> Result<(i32, i32, i32, i32), String> {
    let script = r#"
ObjC.import('AppKit');
(function() {
    var se = Application('System Events');
    var proc = se.processes['PROCESS_NAME'];
    if (proc.windows.length === 0) return 'no-window';

    var pos = proc.windows[0].position();
    var winX = pos[0], winY = pos[1];

    var screens = $.NSScreen.screens;
    var primaryHeight = screens.objectAtIndex(0).frame.size.height;
    // Convert top-left origin (System Events) to bottom-left origin (NSScreen)
    var nsWinY = primaryHeight - winY;

    for (var i = 0; i < screens.count; i++) {
        var screen = screens.objectAtIndex(i);
        var frame = screen.frame;
        if (winX >= frame.origin.x && winX < frame.origin.x + frame.size.width &&
            nsWinY > frame.origin.y && nsWinY <= frame.origin.y + frame.size.height) {
            var vf = screen.visibleFrame;
            var left = Math.round(vf.origin.x);
            var top = Math.round(primaryHeight - vf.origin.y - vf.size.height);
            var right = Math.round(vf.origin.x + vf.size.width);
            var bottom = Math.round(primaryHeight - vf.origin.y);
            return left + ", " + top + ", " + right + ", " + bottom;
        }
    }

    // Fallback: primary screen visible frame
    var vf = screens.objectAtIndex(0).visibleFrame;
    var left = Math.round(vf.origin.x);
    var top = Math.round(primaryHeight - vf.origin.y - vf.size.height);
    var right = Math.round(vf.origin.x + vf.size.width);
    var bottom = Math.round(primaryHeight - vf.origin.y);
    return left + ", " + top + ", " + right + ", " + bottom;
})()
"#
    .replace("PROCESS_NAME", process_name);

    let bounds_str = run_jxa(&script)?;
    if bounds_str == "no-window" {
        return Err("No window found for process".to_string());
    }

    let bounds: Vec<i32> = bounds_str
        .split(", ")
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if bounds.len() != 4 {
        return Err(format!("Unexpected screen bounds: {}", bounds_str));
    }

    Ok((bounds[0], bounds[1], bounds[2], bounds[3]))
}

/// Represents an active companion app window to be positioned.
enum CompanionKind {
    /// Chrome with optional PID and tracked window ID (for real profile mode)
    Chrome { pid: u32, window_id: Option<i64>, is_real_profile: bool },
    /// Cursor companion (found by folder name in window title)
    Cursor,
}

/// Auto-layout session windows: companions on the left, terminal on the right.
/// Screen is divided into N equal columns where N = companions + 1 (terminal).
/// Layout order: Chrome (leftmost) | Cursor | Terminal (rightmost).
///
/// Key design choices:
/// - Finds terminal window by project folder name (not `window 1`) to avoid targeting wrong windows
/// - Uses NSApplicationActivateIgnoringOtherApps WITHOUT AllWindows flag for the terminal,
///   so only the project-specific window comes to front (not all Cursor/VS Code windows)
/// - Exits Chrome fullscreen before repositioning
fn layout_session_windows(_terminal_pid: u32, terminal_app: &str, project_path: &str) -> Result<(), String> {
    let process_name = match terminal_process_name(terminal_app) {
        Some(name) => name,
        None => return Ok(()), // Can't position tmux/unknown
    };

    let folder = project_path.split('/').last().unwrap_or(project_path);

    // Minimize companion instances from other sessions
    minimize_other_companion_instances(project_path);

    // Get screen bounds for the monitor the terminal is currently on
    let (left, top, right, bottom) = screen_bounds_for_process(process_name)?;

    // Collect active companions (ordered: Chrome first, then Cursor)
    // Verify each companion's window actually exists — if manually closed, clean up tracking.
    let mut companions: Vec<CompanionKind> = Vec::new();

    // Check Chrome
    let is_real_profile = read_chrome_profile().is_some();
    if is_real_profile {
        let mut windows = load_chrome_window_ids();
        if let Some(&window_id) = windows.get(project_path) {
            if chrome_window_exists(window_id) {
                if let Some(cpid) = find_main_chrome_pid() {
                    companions.push(CompanionKind::Chrome { pid: cpid, window_id: Some(window_id), is_real_profile: true });
                }
            } else {
                // Window was manually closed — clean up tracking
                log::info!("layout: Chrome window {} was closed, removing from tracking", window_id);
                windows.remove(project_path);
                save_chrome_window_ids(&windows);
                CHROME_URLS.lock().unwrap().remove(project_path);
            }
        }
    } else if let Some(cpid) = chrome_profile_dir(project_path).ok().and_then(|dir| chrome_pid_for_profile(&dir)) {
        companions.push(CompanionKind::Chrome { pid: cpid, window_id: None, is_real_profile: false });
    }

    // Check Cursor (only if terminal is NOT Cursor — no point having Cursor as both terminal and companion)
    if terminal_app != "cursor" && load_cursor_projects().contains(project_path) {
        if cursor_window_exists_for_project(project_path) {
            companions.push(CompanionKind::Cursor);
        } else {
            // Window was manually closed — clean up tracking
            log::info!("layout: Cursor window for {} was closed, removing from tracking", project_path);
            let mut projects = load_cursor_projects();
            projects.remove(project_path);
            save_cursor_projects(&projects);
        }
    }

    // Divide screen into equal columns
    let total_windows = companions.len() as i32 + 1; // +1 for terminal
    let screen_width = right - left;
    let col_width = screen_width / total_windows;

    // Position each companion in its column
    for (i, companion) in companions.iter().enumerate() {
        let col_left = left + (i as i32) * col_width;
        let col_right = col_left + col_width;

        match companion {
            CompanionKind::Chrome { pid: cpid, window_id, is_real_profile: real } => {
                if *real {
                    if let Some(wid) = window_id {
                        let _ = run_applescript(&format!(
                            r#"tell application "Google Chrome"
                                repeat with w in windows
                                    if (id of w as text) is "{wid}" then
                                        set bounds of w to {{{cl}, {ct}, {cr}, {cb}}}
                                        set index of w to 1
                                        exit repeat
                                    end if
                                end repeat
                            end tell"#,
                            wid = wid,
                            cl = col_left, ct = top, cr = col_right, cb = bottom,
                        ));
                    }
                    focus_pid_main_window_only(*cpid);
                } else {
                    activate_pid(*cpid);
                    let _ = run_applescript(&format!(
                        r#"tell application "System Events"
    set chromeProc to first process whose unix id is {cpid}
    set frontmost of chromeProc to true
end tell
delay 0.15
tell application "System Events"
    set chromeProc to first process whose unix id is {cpid}
    try
        set fs to value of attribute "AXFullScreen" of window 1 of chromeProc
        if fs then
            set value of attribute "AXFullScreen" of window 1 of chromeProc to false
            delay 0.5
        end if
    end try
    set position of window 1 of chromeProc to {{{cl}, {ct}}}
    set size of window 1 of chromeProc to {{{cw}, {ch}}}
end tell"#,
                        cpid = cpid,
                        cl = col_left, ct = top, cw = col_right - col_left, ch = bottom - top,
                    ));
                }
            }
            CompanionKind::Cursor => {
                // Position Cursor window by folder name match
                let _ = run_applescript(&format!(
                    r#"tell application "System Events"
    if exists process "Cursor" then
        tell process "Cursor"
            repeat with w in windows
                if name of w contains "{folder}" then
                    perform action "AXRaise" of w
                    set position of w to {{{cl}, {ct}}}
                    set size of w to {{{cw}, {ch}}}
                    exit repeat
                end if
            end repeat
        end tell
    end if
end tell"#,
                    folder = folder,
                    cl = col_left, ct = top, cw = col_right - col_left, ch = bottom - top,
                ));
            }
        }
    }

    // Terminal gets the last (rightmost) column
    let term_left = left + (companions.len() as i32) * col_width;
    let term_width = right - term_left;

    let layout_script = format!(
        r#"tell application "System Events"
    tell process "{proc}"
        set targetWin to missing value
        repeat with w in windows
            if name of w contains "{folder}" then
                set targetWin to w
                exit repeat
            end if
        end repeat

        if targetWin is not missing value then
            perform action "AXRaise" of targetWin
            set position of targetWin to {{{tl}, {tt}}}
            set size of targetWin to {{{tw}, {th}}}
        else
            set position of window 1 to {{{tl}, {tt}}}
            set size of window 1 to {{{tw}, {th}}}
        end if

        return unix id
    end tell
end tell"#,
        proc = process_name,
        folder = folder,
        tl = term_left, tt = top, tw = term_width, th = bottom - top,
    );

    if let Ok(terminal_app_pid_str) = run_applescript(&layout_script) {
        if let Ok(terminal_app_pid) = terminal_app_pid_str.trim().parse::<u32>() {
            focus_pid_main_window_only(terminal_app_pid);
        }
    }

    Ok(())
}

/// Detach Chrome window tracking for a session.
/// Next click on the Chrome icon will open a new window.
#[tauri::command]
pub fn detach_chrome(project_path: String) {
    let mut windows = load_chrome_window_ids();
    windows.remove(&project_path);
    save_chrome_window_ids(&windows);
    CHROME_URLS.lock().unwrap().remove(&project_path);
}

/// Launch (or focus) a Cursor companion window for a session.
/// Cursor windows are tracked per project path in cursor-projects.json.
#[tauri::command]
pub fn launch_cursor(project_path: String) -> Result<(), String> {
    // If Cursor already has a window for this project, raise it
    if cursor_window_exists_for_project(&project_path) {
        terminal::vscode::activate_app_window("Cursor", &project_path)?;
        // Ensure it's tracked
        let mut projects = load_cursor_projects();
        projects.insert(project_path);
        save_cursor_projects(&projects);
        return Ok(());
    }

    // Launch Cursor for this project
    std::process::Command::new("open")
        .arg("-a")
        .arg("Cursor")
        .arg(&project_path)
        .spawn()
        .map_err(|e| format!("Failed to launch Cursor: {}", e))?;

    // Track the project
    let mut projects = load_cursor_projects();
    projects.insert(project_path);
    save_cursor_projects(&projects);

    Ok(())
}

/// Detach Cursor companion tracking for a session.
#[tauri::command]
pub fn detach_cursor(project_path: String) {
    let mut projects = load_cursor_projects();
    projects.remove(&project_path);
    save_cursor_projects(&projects);
}

/// Open a project in Cursor editor (one-shot, not tracked as companion)
#[tauri::command]
pub fn open_in_cursor(project_path: String) -> Result<(), String> {
    std::process::Command::new("open")
        .arg("-a")
        .arg("Cursor")
        .arg(&project_path)
        .spawn()
        .map_err(|e| format!("Failed to open Cursor: {}", e))?;
    Ok(())
}

/// Kill an agent process by PID
#[tauri::command]
pub fn kill_session(pid: u32) -> Result<(), String> {
    use std::process::Command;

    // Use SIGKILL (-9) to forcefully terminate the process
    // SIGTERM often doesn't work for agent processes with child processes
    let output = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("Failed to execute kill command: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Failed to kill process {}: {}", pid, stderr))
    }
}

/// Kill a session and close its attached companion windows (Chrome, Cursor).
#[tauri::command]
pub fn kill_session_and_companions(pid: u32, project_path: String) -> Result<(), String> {
    // Close Chrome companion
    let is_real_profile = read_chrome_profile().is_some();
    if is_real_profile {
        let mut windows = load_chrome_window_ids();
        if let Some(&window_id) = windows.get(&project_path) {
            // Close the specific Chrome window
            let _ = run_applescript(&format!(
                r#"tell application "Google Chrome"
                    repeat with w in windows
                        if (id of w as text) is "{}" then
                            close w
                            exit repeat
                        end if
                    end repeat
                end tell"#,
                window_id
            ));
            windows.remove(&project_path);
            save_chrome_window_ids(&windows);
        }
    } else if let Ok(profile_dir) = chrome_profile_dir(&project_path) {
        if let Some(cpid) = chrome_pid_for_profile(&profile_dir) {
            // Kill the isolated Chrome process
            let _ = std::process::Command::new("kill").arg(cpid.to_string()).output();
        }
    }
    CHROME_URLS.lock().unwrap().remove(&project_path);

    // Close Cursor companion window
    let mut cursor_projects = load_cursor_projects();
    if cursor_projects.remove(&project_path) {
        save_cursor_projects(&cursor_projects);
        let folder = project_path.split('/').last().unwrap_or(&project_path);
        // Close the Cursor window matching this project
        let _ = run_applescript(&format!(
            r#"tell application "System Events"
                if exists process "Cursor" then
                    tell process "Cursor"
                        repeat with w in windows
                            if name of w contains "{}" then
                                -- Use keyboard shortcut to close window (Cmd+W)
                                perform action "AXRaise" of w
                                click menu item "Close Window" of menu "File" of menu bar 1
                                exit repeat
                            end if
                        end repeat
                    end tell
                end if
            end tell"#,
            folder
        ));
    }

    // Kill the agent process itself
    kill_session(pid)
}
