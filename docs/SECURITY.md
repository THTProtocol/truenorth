# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT open a public issue.**
2. Email: hightable.market@gmail.com
3. Include: description, reproduction steps, potential impact.
4. We will respond within 72 hours.

## Security Model

### Authentication
- Bearer token authentication when `TRUENORTH_AUTH_TOKEN` is set
- Health check (`/health`) and Agent Card (`/.well-known/agent.json`) are always public
- All other endpoints require valid token

### API Key Storage
- API keys are loaded from `.env` or environment variables
- Keys are NEVER logged, serialized to disk, or included in error messages
- Keys are passed to provider implementations via reference, never cloned unnecessarily

### WASM Sandbox
- All third-party tools execute in Wasmtime WASM sandbox
- Fuel limits prevent infinite loops (default: 1,000,000 fuel units)
- Capability-based access control: tools declare required capabilities
- No filesystem access unless explicitly granted
- No network access unless explicitly granted

### Memory Security
- All memory data stored locally (SQLite + Tantivy + Markdown)
- No external database dependencies
- Memory content is not filtered for PII by default (user responsibility)
- Obsidian vault sync is read-write to local filesystem only

### Network Security
- All LLM API calls use HTTPS
- TLS for external connections via reqwest with system certificate store
- No custom certificate pinning (deferred to v2)
