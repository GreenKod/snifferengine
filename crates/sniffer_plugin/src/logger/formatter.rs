/// Severity levels for plugin logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Normalizes raw plugin identifiers into short, readable names.
///
/// The engine stays plugin-agnostic: it never enumerates specific plugin
/// names. A `com.sniffer.` namespace prefix is stripped and any remaining
/// namespace segments collapse to the final segment, so
/// `"com.sniffer.anything"` and `"com.example.myplugin"` both reduce to a
/// single readable token.
#[must_use]
pub fn clean_plugin_id(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return "system".to_string();
    }
    let stripped = trimmed.strip_prefix("com.sniffer.").unwrap_or(trimmed);
    stripped
        .rsplit_once('.')
        .map_or_else(|| stripped.to_string(), |(_, last)| last.to_string())
}

/// Cleans redundant prefix tags often emitted by JS plugins (e.g. `[Dock Plugin] `, `[Clock Widget] `)
/// while safely preserving legitimate data structures like JSON arrays (`[1, 2, 3]`).
#[must_use]
pub fn clean_message(msg: &str) -> String {
    let trimmed = msg.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(close_idx) = rest.find(']') {
            let inside = &rest[..close_idx];
            let after = rest[close_idx + 1..].trim_start();
            // Match typical log tags: alphanumeric with spaces, dashes, underscores, colons, or @ symbols,
            // containing at least one alphabetic character, followed by non-empty message content.
            if !after.is_empty()
                && inside.chars().all(|c| {
                    c.is_alphanumeric()
                        || c.is_whitespace()
                        || c == '-'
                        || c == '_'
                        || c == ':'
                        || c == '@'
                })
                && inside.chars().any(char::is_alphabetic)
            {
                let cleaned = after.strip_prefix(':').unwrap_or(after).trim_start();
                return cleaned.to_string();
            }
        }
    }
    trimmed.to_string()
}

/// Formats a single log line according to the user specification:
/// - Info: `# <message> from <plugin>, type info [xN]` or `# info from <plugin> [xN]`
/// - Warn: `# warn: <message> from <plugin> [xN]` or `# warn from <plugin> [xN]`
/// - Error: `# error: <message> from <plugin> [xN]` or `# error from <plugin> [xN]`
#[must_use]
pub fn format_log_line(plugin: &str, level: LogLevel, message: &str, count: u32) -> String {
    let clean_p = clean_plugin_id(plugin);
    let clean_m = clean_message(message);

    let repeat_suffix = if count > 1 {
        format!(" [x{count}]")
    } else {
        String::new()
    };

    match level {
        LogLevel::Info => {
            if clean_m.is_empty() || clean_m.eq_ignore_ascii_case("info") {
                format!("# info from {clean_p}{repeat_suffix}")
            } else {
                format!("# {clean_m} from {clean_p}, type info{repeat_suffix}")
            }
        }
        LogLevel::Warn => {
            if clean_m.is_empty()
                || clean_m.eq_ignore_ascii_case("warn")
                || clean_m.eq_ignore_ascii_case("warning")
            {
                format!("# warn from {clean_p}{repeat_suffix}")
            } else {
                format!("# warn: {clean_m} from {clean_p}{repeat_suffix}")
            }
        }
        LogLevel::Error => {
            if clean_m.is_empty() || clean_m.eq_ignore_ascii_case("error") {
                format!("# error from {clean_p}{repeat_suffix}")
            } else {
                format!("# error: {clean_m} from {clean_p}{repeat_suffix}")
            }
        }
    }
}
