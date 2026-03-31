# TrueNorth REST API Reference

## Base URL

```
http://localhost:8080/api/v1
```

## Authentication

When `TRUENORTH_AUTH_TOKEN` is set, all endpoints (except `/health` and `/.well-known/agent.json`) require:

```
Authorization: Bearer <token>
```

## Endpoints

### Health Check

```
GET /health
```

Response: `200 OK`
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_secs": 3600
}
```

### Submit Task

```
POST /api/v1/task
```

Request:
```json
{
  "prompt": "Research the latest developments in...",
  "session_id": "optional-uuid",
  "execution_mode": "auto",
  "stream": true
}
```

Response (SSE stream when `stream: true`):
```
event: token
data: {"content": "Based on my research..."}

event: tool_call
data: {"tool": "web_search", "input": {"query": "..."}}

event: done
data: {"session_id": "uuid", "tokens_used": 1234}
```

### List Sessions

```
GET /api/v1/sessions
```

Response:
```json
{
  "sessions": [
    {
      "id": "uuid",
      "created_at": "2026-03-31T22:00:00Z",
      "status": "active",
      "task_count": 5
    }
  ]
}
```

### Get Session

```
GET /api/v1/sessions/:id
```

### Cancel Session

```
DELETE /api/v1/sessions/:id
```

### Visual Reasoning Events (WebSocket)

```
GET /api/v1/events/ws
```

Upgrade to WebSocket. Receives JSON events:
```json
{
  "type": "state_transition",
  "session_id": "uuid",
  "from": "Planning",
  "to": "Executing",
  "timestamp": "2026-03-31T22:00:00Z"
}
```

### Visual Reasoning Events (SSE)

```
GET /api/v1/events/sse
```

Server-Sent Events stream of reasoning events.

### List Skills

```
GET /api/v1/skills
```

### List Tools

```
GET /api/v1/tools
```

### Search Memory

```
GET /api/v1/memory/search?q=query&scope=project&limit=10
```

### A2A Agent Card

```
GET /.well-known/agent.json
```

Response:
```json
{
  "name": "TrueNorth",
  "description": "LLM-agnostic AI orchestration harness",
  "version": "0.1.0",
  "capabilities": ["research", "code-review", "reasoning"],
  "skills": ["research-assistant", "code-reviewer", "rcs-debate"],
  "api": {
    "task_endpoint": "/api/v1/task",
    "events_endpoint": "/api/v1/events/ws"
  }
}
```
