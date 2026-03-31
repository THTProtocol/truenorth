//! Runtime initialisation helpers.
//!
//! This module contains two responsibilities:
//! 1. **Tracing** — set up the [`tracing_subscriber`] global subscriber with
//!    an environment-variable filter, overridden by the `--verbose` flag.
//! 2. **Config loading** — read `config.toml` from the path supplied via
//!    `--config`, returning a generic [`serde_json::Value`] so that the CLI
//!    crate avoids a hard dependency on internal config types.

use anyhow::{Context, Result};
use tracing::Level;
use tracing_subscriber::{fmt, EnvFilter};

/// Initialise the global tracing subscriber.
///
/// The filter is resolved in the following order (highest priority first):
///
/// 1. `RUST_LOG` environment variable (if set).
/// 2. The verbosity count from the `--verbose` / `-v` flag:
///    - 0 → `warn`
///    - 1 → `info`
///    - 2 → `debug`
///    - 3+ → `trace`
///
/// Call this exactly once, before any other async work begins.
pub fn init_tracing(verbosity: u8) {
    let default_level = match verbosity {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    // Honour RUST_LOG if set; otherwise fall back to the verbosity-derived
    // level.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level.as_str()));

    // Best-effort: if a subscriber is already installed (e.g. in tests) we
    // silently ignore the error.
    let _ = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_target(verbosity >= 2)
        .with_file(verbosity >= 3)
        .with_line_number(verbosity >= 3)
        .try_init();
}

/// Load the configuration file at `path` and return it as a generic JSON
/// value.
///
/// If the file does not exist or cannot be parsed the function logs a warning
/// and returns a default empty JSON object — it never fails hard, so that all
/// CLI commands work even without a config file present.
///
/// # Errors
///
/// Returns an error only when the file exists but cannot be read due to an OS
/// permission error.  Parse failures are treated as "use defaults" and logged
/// at WARN level.
pub fn load_config(path: &str) -> Result<serde_json::Value> {
    use std::path::Path;

    let p = Path::new(path);

    if !p.exists() {
        tracing::debug!(config_path = %path, "config file not found — using built-in defaults");
        return Ok(serde_json::json!({}));
    }

    let raw = std::fs::read_to_string(p)
        .with_context(|| format!("failed to read config file: {path}"))?;

    // Attempt to parse as TOML and convert to JSON.
    match raw.parse::<toml_minimal::TomlValue>() {
        Ok(toml_val) => {
            let json = toml_to_json(toml_val);
            tracing::debug!(config_path = %path, "config loaded successfully");
            Ok(json)
        }
        Err(e) => {
            tracing::warn!(
                config_path = %path,
                error = %e,
                "config file parse error — using built-in defaults"
            );
            Ok(serde_json::json!({}))
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal TOML → JSON conversion (no extra dep needed beyond serde_json)
// ---------------------------------------------------------------------------

/// A deliberately minimal TOML value type so we can avoid pulling in the full
/// `toml` crate at the CLI layer.  We parse only what we need.
mod toml_minimal {
    use std::fmt;
    use std::str::FromStr;

    /// A value read from TOML (simplified representation).
    #[derive(Debug, Clone)]
    pub enum TomlValue {
        String(String),
        Integer(i64),
        Float(f64),
        Boolean(bool),
        Table(Vec<(String, TomlValue)>),
        #[allow(dead_code)]
        Array(Vec<TomlValue>),
    }

    /// Very lightweight line-by-line TOML parser that handles the subset of
    /// TOML that TrueNorth config files use (flat key=value pairs and
    /// `[section]` headers).  Production use will replace this with the full
    /// `toml` crate once it is in the workspace dependencies.
    #[derive(Debug)]
    pub struct ParseError(pub String);

    impl fmt::Display for ParseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "TOML parse error: {}", self.0)
        }
    }

    impl FromStr for TomlValue {
        type Err = ParseError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut table: Vec<(String, TomlValue)> = Vec::new();
            let mut current_section: Option<String> = None;
            let mut section_entries: Vec<(String, TomlValue)> = Vec::new();

            for line in s.lines() {
                let line = line.trim();

                // Skip blank lines and comments.
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                // Section header.
                if line.starts_with('[') && line.ends_with(']') && !line.starts_with("[[") {
                    // Flush previous section.
                    if let Some(sec) = current_section.take() {
                        table.push((sec, TomlValue::Table(section_entries.clone())));
                        section_entries.clear();
                    }
                    current_section = Some(line[1..line.len() - 1].trim().to_owned());
                    continue;
                }

                // Key = value pair.
                if let Some(eq_pos) = line.find('=') {
                    let key = line[..eq_pos].trim().to_owned();
                    let raw_val = line[eq_pos + 1..].trim();
                    let val = parse_scalar(raw_val);
                    if let Some(_sec) = &current_section {
                        section_entries.push((key, val));
                    } else {
                        table.push((key, val));
                    }
                }
            }

            // Flush last section.
            if let Some(sec) = current_section {
                table.push((sec, TomlValue::Table(section_entries)));
            }

            Ok(TomlValue::Table(table))
        }
    }

    fn parse_scalar(s: &str) -> TomlValue {
        // Quoted string.
        if (s.starts_with('"') && s.ends_with('"'))
            || (s.starts_with('\'') && s.ends_with('\''))
        {
            return TomlValue::String(s[1..s.len() - 1].to_owned());
        }
        // Boolean.
        if s == "true" {
            return TomlValue::Boolean(true);
        }
        if s == "false" {
            return TomlValue::Boolean(false);
        }
        // Integer.
        if let Ok(i) = s.parse::<i64>() {
            return TomlValue::Integer(i);
        }
        // Float.
        if let Ok(f) = s.parse::<f64>() {
            return TomlValue::Float(f);
        }
        // Fallback: treat as unquoted string.
        TomlValue::String(s.to_owned())
    }
}

/// Convert a [`toml_minimal::TomlValue`] into a [`serde_json::Value`].
fn toml_to_json(val: toml_minimal::TomlValue) -> serde_json::Value {
    use toml_minimal::TomlValue;
    match val {
        TomlValue::String(s) => serde_json::Value::String(s),
        TomlValue::Integer(i) => serde_json::json!(i),
        TomlValue::Float(f) => serde_json::json!(f),
        TomlValue::Boolean(b) => serde_json::Value::Bool(b),
        TomlValue::Table(entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                map.insert(k, toml_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        TomlValue::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(toml_to_json).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_tracing_does_not_panic() {
        // Calling init_tracing multiple times must not panic.
        init_tracing(0);
        init_tracing(1);
        init_tracing(3);
    }

    #[test]
    fn test_load_config_missing_file_returns_empty_object() {
        let result = load_config("/nonexistent/path/config.toml").unwrap();
        assert!(result.is_object());
        assert_eq!(result.as_object().unwrap().len(), 0);
    }

    #[test]
    fn test_load_config_real_file() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "port = 9000").unwrap();
        writeln!(tmp, "host = \"0.0.0.0\"").unwrap();

        let result = load_config(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(result["port"], serde_json::json!(9000i64));
        assert_eq!(result["host"], serde_json::json!("0.0.0.0"));
    }
}
