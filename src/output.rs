//! Text vs `--json` formatting for management commands.

use anyhow::Result;
use serde::Serialize;

/// Serialize `value` as pretty JSON with a trailing newline.
pub fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)? + "\n")
}

/// `{"error":"…"}` for `--json` failures (stdout).
pub fn error_json(err: &anyhow::Error) -> String {
    let payload = serde_json::json!({ "error": format!("{err:#}") });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{\"error\":\"unknown error\"}".into())
        + "\n"
}

/// JSON when `json` is set, otherwise `text`.
pub fn pick<T: Serialize>(json: bool, value: &T, text: String) -> Result<String> {
    if json {
        to_json(value)
    } else {
        Ok(text)
    }
}
