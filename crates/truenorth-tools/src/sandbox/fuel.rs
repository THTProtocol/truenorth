//! Per-invocation fuel metering for WASM sandboxes.
//!
//! [`FuelMeter`] tracks CPU fuel consumption for a single WASM invocation,
//! emitting a warning at 80% usage and enforcing a hard stop at 100%.

use tracing::warn;

/// Tracks fuel consumption for a single WASM invocation.
///
/// Wasmtime's fuel metering assigns a fixed budget to each execution. This
/// tracker provides convenient percentage-based thresholds so that:
/// - Callers can warn when a module is approaching its budget.
/// - Post-execution reporting can include percentage utilisation.
#[derive(Debug, Clone)]
pub struct FuelMeter {
    /// The total fuel budget for this invocation.
    budget: u64,
    /// Fuel consumed so far (updated after execution).
    consumed: u64,
    /// Whether the 80% warning has already been emitted.
    warned: bool,
}

impl FuelMeter {
    /// Creates a new `FuelMeter` with the given budget.
    ///
    /// # Arguments
    /// * `budget` — the total fuel units available for this invocation.
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            consumed: 0,
            warned: false,
        }
    }

    /// Records that `units` of fuel were consumed.
    ///
    /// Emits a `WARN` trace message if utilisation has crossed 80%.
    /// Subsequent calls after the warning threshold do not re-emit.
    ///
    /// # Arguments
    /// * `units` — fuel units consumed since the last `record` call.
    /// * `tool_name` — name of the tool being metered (for log context).
    pub fn record(&mut self, units: u64, tool_name: &str) {
        self.consumed = self.consumed.saturating_add(units);

        if !self.warned && self.utilisation_pct() >= 80.0 {
            warn!(
                tool_name,
                consumed = self.consumed,
                budget = self.budget,
                utilisation_pct = self.utilisation_pct(),
                "WASM fuel at 80% — approaching execution limit"
            );
            self.warned = true;
        }
    }

    /// Returns `true` if the fuel budget has been fully exhausted.
    ///
    /// The WASM execution should be halted immediately when this returns `true`.
    pub fn is_exhausted(&self) -> bool {
        self.consumed >= self.budget
    }

    /// Returns the percentage of the fuel budget consumed (0.0–100.0).
    pub fn utilisation_pct(&self) -> f64 {
        if self.budget == 0 {
            return 100.0;
        }
        (self.consumed as f64 / self.budget as f64) * 100.0
    }

    /// Returns the number of fuel units consumed so far.
    pub fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Returns the total fuel budget.
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Returns the remaining fuel (saturating at 0).
    pub fn remaining(&self) -> u64 {
        self.budget.saturating_sub(self.consumed)
    }

    /// Produces a human-readable summary suitable for `ReasoningEvent` metadata.
    pub fn summary(&self) -> FuelSummary {
        FuelSummary {
            budget: self.budget,
            consumed: self.consumed,
            remaining: self.remaining(),
            utilisation_pct: self.utilisation_pct(),
            exhausted: self.is_exhausted(),
        }
    }
}

/// A snapshot of fuel consumption at the end of an invocation.
///
/// Included in `ReasoningEvent` metadata for observability.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FuelSummary {
    /// Total fuel budget for the invocation.
    pub budget: u64,
    /// Fuel units actually consumed.
    pub consumed: u64,
    /// Remaining fuel (budget - consumed).
    pub remaining: u64,
    /// Percentage consumed (0.0–100.0).
    pub utilisation_pct: f64,
    /// Whether the budget was exhausted.
    pub exhausted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utilisation_below_80_no_warning() {
        let mut meter = FuelMeter::new(100);
        meter.record(50, "test_tool");
        assert_eq!(meter.utilisation_pct(), 50.0);
        assert!(!meter.is_exhausted());
        assert!(!meter.warned);
    }

    #[test]
    fn test_utilisation_at_80_warns() {
        let mut meter = FuelMeter::new(100);
        meter.record(80, "test_tool");
        assert!(meter.warned);
        assert!(!meter.is_exhausted());
    }

    #[test]
    fn test_exhausted() {
        let mut meter = FuelMeter::new(100);
        meter.record(100, "test_tool");
        assert!(meter.is_exhausted());
        assert_eq!(meter.remaining(), 0);
    }

    #[test]
    fn test_saturation() {
        let mut meter = FuelMeter::new(100);
        meter.record(200, "test_tool");
        assert!(meter.is_exhausted());
        assert_eq!(meter.consumed(), 200);
        assert_eq!(meter.remaining(), 0);
    }

    #[test]
    fn test_summary() {
        let mut meter = FuelMeter::new(1000);
        meter.record(250, "tool");
        let summary = meter.summary();
        assert_eq!(summary.budget, 1000);
        assert_eq!(summary.consumed, 250);
        assert_eq!(summary.remaining, 750);
        assert!((summary.utilisation_pct - 25.0).abs() < 0.001);
        assert!(!summary.exhausted);
    }
}
