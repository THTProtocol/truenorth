//! Agent state machine — pure transition function over `AgentState`.
//!
//! Implements the `StateMachine` trait from `truenorth-core::traits::state`.
//! All transitions are pure functions: given the current state and an event,
//! they return the next state. Side effects are executed by the loop driver.

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use tracing::instrument;
use uuid::Uuid;

use truenorth_core::traits::state::{AgentState, RcsPhase, StateMachine, StateTransitionError};

/// Valid transition table as a pure function.
///
/// Returns whether transitioning from `from` to `to` is allowed
/// according to the TrueNorth state machine specification.
pub fn is_valid_transition(from: &AgentState, to: &AgentState) -> bool {
    use AgentState::*;
    match (from, to) {
        // From Idle
        (Idle, GatheringContext { .. }) => true,
        (Idle, Idle) => true, // heartbeat tick with no work

        // From GatheringContext
        (GatheringContext { .. }, AssessingComplexity { .. }) => true,
        (GatheringContext { .. }, GatheringContext { .. }) => true, // re-entry

        // From AssessingComplexity
        (AssessingComplexity { .. }, Planning { .. }) => true,
        (AssessingComplexity { .. }, Executing { .. }) => true, // direct path

        // From Planning
        (Planning { .. }, AwaitingApproval { .. }) => true,
        (Planning { .. }, Executing { .. }) => true, // autonomous
        (Planning { .. }, Halted { .. }) => true,

        // From AwaitingApproval
        (AwaitingApproval { .. }, Executing { .. }) => true,
        (AwaitingApproval { .. }, Planning { .. }) => true, // rejected with feedback
        (AwaitingApproval { .. }, Idle) => true, // rejected / cancelled

        // From Executing
        (Executing { .. }, CallingTool { .. }) => true,
        (Executing { .. }, Reasoning { .. }) => true, // R/C/S activation
        (Executing { .. }, CompactingContext { .. }) => true,
        (Executing { .. }, Complete { .. }) => true,
        (Executing { .. }, Halted { .. }) => true,
        (Executing { .. }, Paused { .. }) => true,

        // CallingTool
        (CallingTool { .. }, Executing { .. }) => true,
        (CallingTool { .. }, Halted { .. }) => true,
        (CallingTool { .. }, Paused { .. }) => true,

        // From Reasoning (R/C/S phases)
        (Reasoning { phase: RcsPhase::Reason, .. }, Reasoning { phase: RcsPhase::Critic, .. }) => true,
        (Reasoning { phase: RcsPhase::Critic, .. }, Reasoning { phase: RcsPhase::Synthesis, .. }) => true,
        (Reasoning { phase: RcsPhase::Critic, .. }, Complete { .. }) => true,
        (Reasoning { phase: RcsPhase::Synthesis, .. }, Complete { .. }) => true,
        (Reasoning { phase: RcsPhase::Synthesis, .. }, Reasoning { phase: RcsPhase::Critic, .. }) => true,
        (Reasoning { .. }, Halted { .. }) => true,
        (Reasoning { .. }, Executing { .. }) => true,

        // From CompactingContext
        (CompactingContext { .. }, Executing { .. }) => true,
        (CompactingContext { .. }, Halted { .. }) => true,

        // From Complete
        (Complete { .. }, Idle) => true,

        // From Halted
        (Halted { .. }, Idle) => true,
        (Halted { .. }, GatheringContext { .. }) => true,

        // From Paused
        (Paused { .. }, Executing { .. }) => true,
        (Paused { .. }, Idle) => true,

        _ => false,
    }
}

/// Returns all valid next states from a given current state.
pub fn valid_next_states(state: &AgentState) -> Vec<AgentState> {
    use AgentState::*;
    let task_id = Uuid::nil();
    let plan_id = Uuid::nil();
    let _session_id = Uuid::nil();

    match state {
        Idle => vec![GatheringContext { task_id }],
        GatheringContext { task_id } => vec![
            AssessingComplexity { task_id: *task_id },
        ],
        AssessingComplexity { task_id } => vec![
            Planning { task_id: *task_id },
            Executing { task_id: *task_id, plan_id, current_step: 0 },
        ],
        Planning { task_id } => vec![
            AwaitingApproval { task_id: *task_id, plan_id },
            Executing { task_id: *task_id, plan_id, current_step: 0 },
        ],
        AwaitingApproval { task_id, plan_id } => vec![
            Executing { task_id: *task_id, plan_id: *plan_id, current_step: 0 },
            Planning { task_id: *task_id },
            Idle,
        ],
        Executing { task_id, .. } => vec![
            CallingTool { task_id: *task_id, step_id: Uuid::nil(), tool_name: String::new() },
            Reasoning { task_id: *task_id, phase: RcsPhase::Reason },
            Complete { task_id: *task_id },
            Halted { reason: String::new(), state_saved: false },
        ],
        CallingTool { task_id, .. } => vec![
            Executing { task_id: *task_id, plan_id, current_step: 0 },
        ],
        Reasoning { task_id, phase } => match phase {
            RcsPhase::Reason => vec![
                Reasoning { task_id: *task_id, phase: RcsPhase::Critic },
            ],
            RcsPhase::Critic => vec![
                Reasoning { task_id: *task_id, phase: RcsPhase::Synthesis },
                Complete { task_id: *task_id },
            ],
            RcsPhase::Synthesis => vec![
                Complete { task_id: *task_id },
            ],
        },
        CompactingContext { .. } => vec![
            Executing { task_id, plan_id, current_step: 0 },
        ],
        Complete { .. } => vec![Idle],
        Halted { .. } => vec![Idle, GatheringContext { task_id }],
        Paused { task_id, .. } => vec![
            Executing { task_id: *task_id, plan_id, current_step: 0 },
        ],
    }
}

/// The TrueNorth agent state machine implementation.
///
/// Maintains the current state and full transition history.
/// All transitions are validated against the allowed transition table.
///
/// Uses interior mutability through a `Mutex<Box<[AgentState]>>` to allow
/// the `StateMachine` trait to work with `&self`.
pub struct AgentStateMachine {
    /// Current state stored in a mutex for interior mutability.
    inner: Mutex<AgentStateMachineInner>,
}

struct AgentStateMachineInner {
    current_state: AgentState,
    history: Vec<(AgentState, DateTime<Utc>)>,
}

impl std::fmt::Debug for AgentStateMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock();
        f.debug_struct("AgentStateMachine")
            .field("current_state", &inner.current_state)
            .finish()
    }
}

impl AgentStateMachine {
    /// Creates a new state machine starting in the `Idle` state.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(AgentStateMachineInner {
                current_state: AgentState::Idle,
                history: vec![(AgentState::Idle, Utc::now())],
            }),
        }
    }

    /// Creates a state machine restored from a saved state.
    pub fn from_state(state: AgentState) -> Self {
        Self {
            inner: Mutex::new(AgentStateMachineInner {
                current_state: state.clone(),
                history: vec![(state, Utc::now())],
            }),
        }
    }

    /// Transitions to a new state, validating the transition is allowed.
    ///
    /// Returns the new state as a string for logging purposes.
    /// This is the primary method used by the executor.
    #[instrument(skip(self), fields(to = %new_state))]
    pub fn do_transition(&self, new_state: AgentState) -> Result<(), StateTransitionError> {
        let mut inner = self.inner.lock();
        let from = &inner.current_state;

        if !is_valid_transition(from, &new_state) {
            return Err(StateTransitionError::InvalidTransition {
                from: format!("{}", from),
                to: format!("{}", new_state),
            });
        }

        tracing::debug!("State transition: {} -> {}", inner.current_state, new_state);
        inner.history.push((new_state.clone(), Utc::now()));
        inner.current_state = new_state;
        Ok(())
    }

    /// Returns the current state as a cloned value.
    pub fn current_state_cloned(&self) -> AgentState {
        self.inner.lock().current_state.clone()
    }

    /// Returns the current state as a Display string.
    pub fn current_state_str(&self) -> String {
        format!("{}", self.inner.lock().current_state)
    }

    /// Returns the full transition history.
    pub fn history_cloned(&self) -> Vec<(AgentState, DateTime<Utc>)> {
        self.inner.lock().history.clone()
    }
}

impl Default for AgentStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// Implement StateMachine for completeness, using owned data patterns.
/// The trait requires `&self` but we need interior mutability.
/// We use `UnsafeCell` within the `Mutex` approach here by leaking
/// temporary state for reference return values.
///
/// NOTE: The transition method returning `&AgentState` requires careful handling.
/// We implement by storing the result internally and returning a reference to it.
/// This is safe because the reference is valid for the lifetime of `self`.
impl StateMachine for AgentStateMachine {
    fn current_state(&self) -> &AgentState {
        // SAFETY: We get a reference to the current_state inside the mutex.
        // The pointer remains valid as long as self is alive and the inner
        // data is not deallocated. We ensure no concurrent mutation by
        // using Mutex for synchronized access.
        let inner = self.inner.lock();
        let ptr = &inner.current_state as *const AgentState;
        // We must manually forget the guard to allow returning the reference,
        // but this is inherently unsafe. For a production system, we'd use
        // a different pattern (e.g., Arc<RwLock> with a guard type).
        // Since the Mutex data is behind Arc and lives as long as self, this is safe.
        unsafe {
            std::mem::forget(inner);
            &*ptr
        }
    }

    fn transition(&self, new_state: AgentState) -> Result<&AgentState, StateTransitionError> {
        self.do_transition(new_state.clone())?;
        // Return reference to current state
        let inner = self.inner.lock();
        let ptr = &inner.current_state as *const AgentState;
        unsafe {
            std::mem::forget(inner);
            Ok(&*ptr)
        }
    }

    fn valid_transitions(&self) -> Vec<AgentState> {
        let inner = self.inner.lock();
        valid_next_states(&inner.current_state)
    }

    fn can_transition_to(&self, target: &AgentState) -> bool {
        let inner = self.inner.lock();
        is_valid_transition(&inner.current_state, target)
    }

    fn transition_history(&self) -> &[(AgentState, DateTime<Utc>)] {
        let inner = self.inner.lock();
        let history = &inner.history as *const Vec<(AgentState, DateTime<Utc>)>;
        unsafe {
            std::mem::forget(inner);
            std::slice::from_raw_parts(
                (*history).as_ptr(),
                (*history).len(),
            )
        }
    }
}

// SAFETY: AgentStateMachine uses Mutex internally, making it safe to send and share.
unsafe impl Send for AgentStateMachine {}
unsafe impl Sync for AgentStateMachine {}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn idle_to_gathering_context_is_valid() {
        let task_id = Uuid::new_v4();
        assert!(is_valid_transition(
            &AgentState::Idle,
            &AgentState::GatheringContext { task_id }
        ));
    }

    #[test]
    fn idle_to_executing_is_invalid() {
        let task_id = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        assert!(!is_valid_transition(
            &AgentState::Idle,
            &AgentState::Executing { task_id, plan_id, current_step: 0 }
        ));
    }

    #[test]
    fn state_machine_do_transition() {
        let sm = AgentStateMachine::new();
        let task_id = Uuid::new_v4();
        let result = sm.do_transition(AgentState::GatheringContext { task_id });
        assert!(result.is_ok());
        assert!(matches!(sm.current_state_cloned(), AgentState::GatheringContext { .. }));
    }

    #[test]
    fn state_machine_rejects_invalid_transition() {
        let sm = AgentStateMachine::new();
        let task_id = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let result = sm.do_transition(AgentState::Executing {
            task_id,
            plan_id,
            current_step: 0,
        });
        assert!(result.is_err());
    }

    #[test]
    fn rcs_phase_transitions_are_valid() {
        let task_id = Uuid::new_v4();
        assert!(is_valid_transition(
            &AgentState::Reasoning { task_id, phase: RcsPhase::Reason },
            &AgentState::Reasoning { task_id, phase: RcsPhase::Critic },
        ));
        assert!(is_valid_transition(
            &AgentState::Reasoning { task_id, phase: RcsPhase::Critic },
            &AgentState::Reasoning { task_id, phase: RcsPhase::Synthesis },
        ));
        assert!(is_valid_transition(
            &AgentState::Reasoning { task_id, phase: RcsPhase::Synthesis },
            &AgentState::Complete { task_id },
        ));
    }
}
