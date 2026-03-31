//! Capability-based access control for WASM sandbox instances.
//!
//! A [`CapabilitySet`] describes exactly what an executing WASM module is
//! allowed to do. No capability = no access. Every filesystem path, network
//! domain, and system resource must be explicitly granted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use truenorth_core::types::config::SandboxConfig;

/// The complete set of capabilities granted to a WASM module instance.
///
/// Capability-based security: a module can only access exactly what is
/// explicitly listed here. The empty `CapabilitySet::none()` is the
/// most restrictive baseline — all grants must be explicitly added.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySet {
    /// Filesystem paths the module may read from.
    ///
    /// Access is checked by prefix: the module may read any file under any
    /// of these paths (and their descendants).
    pub filesystem_read: Vec<PathBuf>,

    /// Filesystem paths the module may write to.
    ///
    /// Write access is checked by prefix: the module may create or overwrite
    /// files under any of these paths.
    pub filesystem_write: Vec<PathBuf>,

    /// Allowlisted hostnames the module may make HTTP requests to.
    ///
    /// An empty list means no network access. Matching is by exact hostname
    /// or suffix (e.g. `"example.com"` also allows `"api.example.com"`).
    pub network_allow: Vec<String>,

    /// Maximum linear memory for the module instance, in bytes.
    ///
    /// Defaults to 64 MiB. Wasmtime enforces this limit at instantiation time.
    pub max_memory_bytes: usize,

    /// CPU fuel budget for the entire invocation.
    ///
    /// Each Wasm instruction consumes fuel. When exhausted, execution traps
    /// with `OutOfFuel`. Defaults to 10 million units.
    pub max_fuel: u64,

    /// Wall-clock execution timeout in milliseconds.
    ///
    /// Enforced via `tokio::time::timeout` wrapping the entire execution.
    pub max_execution_ms: u64,

    /// Whether the module may access environment variables.
    pub allow_env: bool,

    /// Whether the module may spawn child processes.
    pub allow_subprocess: bool,

    /// Whether the module may read the system clock.
    pub allow_clock: bool,

    /// Whether the module may generate random numbers via the WASI RNG.
    pub allow_random: bool,
}

impl CapabilitySet {
    /// Returns the most restrictive capability set — zero permissions.
    ///
    /// Use this as the baseline and add grants explicitly.
    pub fn none() -> Self {
        Self {
            filesystem_read: vec![],
            filesystem_write: vec![],
            network_allow: vec![],
            max_memory_bytes: 64 * 1024 * 1024,
            max_fuel: 10_000_000,
            max_execution_ms: 30_000,
            allow_env: false,
            allow_subprocess: false,
            allow_clock: true,
            allow_random: true,
        }
    }

    /// Returns a standard sandboxed capability set for untrusted third-party tools.
    ///
    /// Grants read access to `workspace_read` and write access to `outputs_write`.
    /// Network access must be explicitly added per-tool via [`grant_network`].
    ///
    /// # Arguments
    /// * `workspace_read` — directory the module may read from.
    /// * `outputs_write` — directory the module may write to.
    pub fn sandboxed(workspace_read: PathBuf, outputs_write: PathBuf) -> Self {
        Self {
            filesystem_read: vec![workspace_read],
            filesystem_write: vec![outputs_write],
            ..Self::none()
        }
    }

    /// Constructs a `CapabilitySet` from a [`SandboxConfig`].
    ///
    /// The config-derived set has no filesystem or network grants by default —
    /// those must be added per-tool based on what the tool actually needs.
    pub fn from_config(config: &SandboxConfig) -> Self {
        Self {
            filesystem_read: vec![],
            filesystem_write: vec![],
            network_allow: vec![],
            max_memory_bytes: config.max_memory_bytes,
            max_fuel: config.max_fuel,
            max_execution_ms: config.max_execution_ms,
            allow_env: false,
            allow_subprocess: false,
            allow_clock: config.allow_clock,
            allow_random: config.allow_random,
        }
    }

    /// Grants read access to an additional filesystem path.
    pub fn grant_read(mut self, path: PathBuf) -> Self {
        self.filesystem_read.push(path);
        self
    }

    /// Grants write access to an additional filesystem path.
    pub fn grant_write(mut self, path: PathBuf) -> Self {
        self.filesystem_write.push(path);
        self
    }

    /// Adds a network allowlist entry.
    ///
    /// Matching is by hostname or hostname suffix.
    pub fn grant_network(mut self, domain: String) -> Self {
        self.network_allow.push(domain);
        self
    }

    /// Returns `true` if reading `path` is within the granted filesystem read paths.
    ///
    /// A path is allowed if it is equal to or a descendant of any entry in
    /// `filesystem_read`.
    pub fn allows_read(&self, path: &Path) -> bool {
        self.filesystem_read
            .iter()
            .any(|allowed| is_path_within(path, allowed))
    }

    /// Returns `true` if writing to `path` is within the granted filesystem write paths.
    pub fn allows_write(&self, path: &Path) -> bool {
        self.filesystem_write
            .iter()
            .any(|allowed| is_path_within(path, allowed))
    }

    /// Returns `true` if making an HTTP request to `domain` is allowed.
    ///
    /// Matches exact hostname or suffix (e.g., `"example.com"` allows
    /// `"api.example.com"` and `"example.com"` itself).
    pub fn allows_network(&self, domain: &str) -> bool {
        if self.network_allow.is_empty() {
            return false;
        }
        let domain = domain.to_lowercase();
        self.network_allow.iter().any(|allowed| {
            let allowed = allowed.to_lowercase();
            domain == allowed || domain.ends_with(&format!(".{}", allowed))
        })
    }
}

/// Checks whether `path` is equal to or a descendant of `base`.
fn is_path_within(path: &Path, base: &Path) -> bool {
    // Canonicalize would require the paths to exist; use lexicographic prefix check.
    path.starts_with(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allows_read() {
        let caps = CapabilitySet::none().grant_read(PathBuf::from("/workspace"));
        assert!(caps.allows_read(Path::new("/workspace/file.txt")));
        assert!(caps.allows_read(Path::new("/workspace/subdir/file.txt")));
        assert!(!caps.allows_read(Path::new("/etc/passwd")));
        assert!(!caps.allows_read(Path::new("/workspacetoo/file.txt")));
    }

    #[test]
    fn test_allows_write() {
        let caps = CapabilitySet::none().grant_write(PathBuf::from("/workspace/output"));
        assert!(caps.allows_write(Path::new("/workspace/output/result.txt")));
        assert!(!caps.allows_write(Path::new("/workspace/src/main.rs")));
    }

    #[test]
    fn test_allows_network() {
        let caps = CapabilitySet::none().grant_network("example.com".to_string());
        assert!(caps.allows_network("example.com"));
        assert!(caps.allows_network("api.example.com"));
        assert!(!caps.allows_network("evil.com"));
        assert!(!caps.allows_network("notexample.com"));
    }

    #[test]
    fn test_none_denies_everything() {
        let caps = CapabilitySet::none();
        assert!(!caps.allows_read(Path::new("/")));
        assert!(!caps.allows_write(Path::new("/")));
        assert!(!caps.allows_network("localhost"));
    }
}
