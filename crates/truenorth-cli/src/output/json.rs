//! Machine-readable JSON output.
//!
//! When the user passes `--format json` all command output is emitted as
//! pretty-printed JSON to **stdout**.  This makes it easy to pipe
//! `truenorth` output into `jq` or other tools.

use serde_json::Value;

/// Emit a [`serde_json::Value`] as pretty-printed JSON to stdout.
///
/// # Example
///
/// ```
/// let v = serde_json::json!({ "status": "ok", "version": "0.1.0" });
/// truenorth_cli::output::json::print_json(&v);
/// ```
pub fn print_json(value: &Value) {
    // `to_string_pretty` never fails for well-formed Value trees.
    println!("{}", serde_json::to_string_pretty(value).unwrap_or_else(|e| {
        format!("{{\"error\": \"serialisation failed: {e}\"}}")
    }));
}

/// Build a standard JSON response envelope.
///
/// All stub commands use this helper so that JSON output is consistent.
pub fn make_response(status: &str, message: &str) -> Value {
    serde_json::json!({
        "status": status,
        "message": message,
    })
}

/// Build a JSON envelope containing structured data alongside a status field.
pub fn make_data_response(status: &str, data: Value) -> Value {
    serde_json::json!({
        "status": status,
        "data": data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_response() {
        let v = make_response("ok", "hello");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["message"], "hello");
    }

    #[test]
    fn test_make_data_response() {
        let data = serde_json::json!({"port": 8080});
        let v = make_data_response("ok", data);
        assert_eq!(v["status"], "ok");
        assert_eq!(v["data"]["port"], 8080);
    }

    #[test]
    fn test_print_json_does_not_panic() {
        let v = serde_json::json!({ "key": "value", "num": 42 });
        print_json(&v); // writes to stdout
    }
}
