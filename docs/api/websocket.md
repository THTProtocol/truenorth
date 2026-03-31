# WebSocket Protocol

## Connection

```
ws://localhost:8080/api/v1/events/ws
```

## Message Format

All messages are JSON objects with a `type` field:

```json
{
  "type": "event_type",
  "session_id": "uuid",
  "timestamp": "ISO-8601",
  ...payload
}
```

## Event Types

| Type | Description |
|------|-------------|
| `task_received` | New task submitted |
| `plan_created` | Execution plan generated (includes Mermaid diagram) |
| `state_transition` | Agent state changed |
| `step_started` | Execution step beginning |
| `step_completed` | Execution step finished |
| `tool_called` | Tool invocation started |
| `tool_result` | Tool result received |
| `llm_request_sent` | LLM API call initiated |
| `llm_response_received` | LLM response complete |
| `memory_stored` | Memory entry written |
| `memory_retrieved` | Memory search performed |
| `rcs_reason_complete` | R/C/S Reason phase done |
| `rcs_critic_complete` | R/C/S Critic phase done |
| `rcs_synthesis_complete` | R/C/S Synthesis phase done |
| `deviation_detected` | Plan deviation detected |
| `error` | Error occurred |
| `session_complete` | Session finished |

## Client Commands

Clients can send commands:

```json
{"command": "subscribe", "session_id": "uuid"}
{"command": "unsubscribe", "session_id": "uuid"}
{"command": "replay", "session_id": "uuid", "from_event": 0}
```
