use super::model::SessionStatus;

/// Check if content array contains only "thinking" blocks (no text or tool_use).
/// During extended thinking, Claude writes assistant messages with only thinking blocks.
/// These should not count as substantive content for status determination.
pub fn is_thinking_only(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        !arr.is_empty()
            && arr.iter().all(|item| {
                item.get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "thinking")
                    .unwrap_or(false)
            })
    } else {
        false
    }
}

/// Check if message content contains a tool_use block
pub fn has_tool_use(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        arr.iter().any(|item| {
            item.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "tool_use")
                .unwrap_or(false)
        })
    } else {
        false
    }
}

/// Check if message content contains a tool_result block
pub fn has_tool_result(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        arr.iter().any(|item| {
            item.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "tool_result")
                .unwrap_or(false)
        })
    } else {
        false
    }
}

/// Extract text content from a message content value
fn extract_text_content(content: &serde_json::Value) -> &str {
    match content {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Array(arr) => {
            // Find first text block
            arr.iter().find_map(|v| {
                v.get("text").and_then(|t| t.as_str())
            }).unwrap_or("")
        }
        _ => "",
    }
}

/// Check if message content indicates an interrupted request (user pressed Escape)
pub fn is_interrupted_request(content: &serde_json::Value) -> bool {
    let text = extract_text_content(content);
    text.contains("[Request interrupted by user]")
}

/// Check if message content is a local slash command that doesn't trigger Claude response
/// These commands are handled locally by Claude Code and don't require thinking
pub fn is_local_slash_command(content: &serde_json::Value) -> bool {
    let text = extract_text_content(content);
    let trimmed = text.trim();

    // Detect XML-wrapped command format from Claude Code:
    // <command-name>/clear</command-name>...<command-message>clear</command-message>...
    // Also detect command output/caveat messages which are part of local command execution:
    // <local-command-stdout>...</local-command-stdout>
    // <local-command-caveat>...</local-command-caveat>
    if trimmed.starts_with("<local-command-stdout>") || trimmed.starts_with("<local-command-caveat>") {
        return true;
    }

    // Extract command name from <command-name>/cmd</command-name> format
    let command_text = if let Some(rest) = trimmed.strip_prefix("<command-name>") {
        if let Some(cmd) = rest.split("</command-name>").next() {
            cmd.trim()
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    // Local commands that don't trigger Claude to think
    // These are handled by the CLI itself
    let local_commands = [
        "/clear",
        "/compact",
        "/help",
        "/config",
        "/cost",
        "/doctor",
        "/init",
        "/login",
        "/logout",
        "/memory",
        "/model",
        "/permissions",
        "/pr-comments",
        "/review",
        "/status",
        "/terminal-setup",
        "/vim",
    ];

    local_commands.iter().any(|cmd| {
        command_text == *cmd || command_text.starts_with(&format!("{} ", cmd))
    })
}

/// Returns sort priority for status (lower = higher priority in list)
/// Active sessions (thinking/processing) appear first, then waiting, then idle
pub fn status_sort_priority(status: &SessionStatus) -> u8 {
    match status {
        SessionStatus::Thinking => 0,    // Active - Claude is working - show first
        SessionStatus::Processing => 0,  // Active - tool is running - show first
        SessionStatus::Compacting => 0,  // Active - compressing context - show first
        SessionStatus::Waiting => 1,     // Needs attention - show second
        SessionStatus::Idle => 2,        // Inactive - show last
    }
}

/// Determine session status based on the last message in the conversation
///
/// Status is determined from message content + file age + CPU usage:
/// - assistant with tool_use + file active (< 8s) or CPU high -> Processing
/// - assistant with tool_use + file quiet + CPU low -> Waiting (blocked on user)
/// - assistant text-only + file quiet (> 3s) -> Idle (Claude finished)
/// - user message -> Thinking (Claude is generating a response)
/// - user with tool_result -> Thinking (Claude is processing tool output)
/// - local slash command or interrupted -> Idle (no Claude response expected)
pub fn determine_status(
    last_msg_type: Option<&str>,
    has_tool_use: bool,
    _has_tool_result: bool,
    is_local_command: bool,
    is_interrupted: bool,
    file_age_secs: Option<f32>,
    cpu_usage: f32,
) -> SessionStatus {
    // Two thresholds: tight for text-only (quick Idle), generous for tool_use
    let file_recently_modified = file_age_secs.map(|age| age < 3.0).unwrap_or(false);
    let file_active_for_tool = file_age_secs.map(|age| age < 8.0).unwrap_or(false);
    let cpu_active = cpu_usage > 5.0;

    match last_msg_type {
        Some("assistant") => {
            if has_tool_use {
                if file_active_for_tool || cpu_active {
                    // Tool is actively running: file was modified within 8s,
                    // or process is using significant CPU (tool execution, streaming)
                    SessionStatus::Processing
                } else {
                    // Tool_use sent, file quiet for 8+ seconds, low CPU
                    // -> waiting for user permission/answer
                    SessionStatus::Waiting
                }
            } else if file_recently_modified {
                // Text response but file is still being written to
                // (streaming, compacting, or about to send tool_use)
                SessionStatus::Processing
            } else {
                // Assistant sent a text response and file is quiet - done, no pending questions
                SessionStatus::Idle
            }
        }
        Some("user") => {
            if is_local_command || is_interrupted {
                // Local slash commands and interrupted requests don't trigger Claude
                SessionStatus::Idle
            } else {
                // User sent a message or tool result - Claude is working
                SessionStatus::Thinking
            }
        }
        _ => {
            // Couldn't determine message type (e.g., only progress entries in lookback)
            if file_recently_modified {
                SessionStatus::Processing
            } else {
                SessionStatus::Idle
            }
        }
    }
}
