# TrueNorth Deployment Guide

> **Version**: 0.1.0  
> **Last updated**: 2026-03-31  
> **Audience**: DevOps engineers and developers deploying TrueNorth in any environment.

---

## Table of Contents

1. [Local Development](#1-local-development)
2. [Docker Deployment](#2-docker-deployment)
3. [Fly.io Deployment](#3-flyio-deployment)
4. [Railway Deployment](#4-railway-deployment)
5. [Environment Variables Reference](#5-environment-variables-reference)
6. [Configuration File Reference](#6-configuration-file-reference)
7. [TLS and Reverse Proxy Setup](#7-tls-and-reverse-proxy-setup)
8. [Monitoring and Health Checks](#8-monitoring-and-health-checks)
9. [Backup and Migration](#9-backup-and-migration)

---

## 1. Local Development

### From Source

```bash
# Clone and configure
git clone https://github.com/THTProtocol/truenorth.git
cd truenorth
cp .env.example .env
cp config.toml.example config.toml

# Set at least one API key in .env
echo "ANTHROPIC_API_KEY=sk-ant-..." >> .env

# Build (debug — fast compile)
cargo build

# Run a task
./target/debug/truenorth run --task "Hello world"

# Start the web server
./target/debug/truenorth serve --port 8080

# Start with verbose logging
RUST_LOG=debug ./target/debug/truenorth serve --port 8080
```

### Data Directory

By default, all TrueNorth data is stored in `~/.truenorth/`:

```
~/.truenorth/
├── config.toml         ← Configuration (if not using project-local config.toml)
├── sessions/           ← Serialized session state
├── memory/             ← Three-tier memory databases
│   ├── project.db
│   ├── identity.db
│   ├── tantivy_index/
│   └── vault/          ← Obsidian-compatible Markdown
├── skills/             ← Installed skill files
└── models/             ← Embedding model cache (fastembed)
```

Override the data directory:
```bash
TRUENORTH_DATA_DIR=/opt/truenorth ./target/release/truenorth serve
```

Or in `config.toml`:
```toml
data_dir = "/opt/truenorth"
```

### CLI Commands Reference

```bash
# Run a single task and exit
truenorth run --task "Your task here"
truenorth run --task "Your task" --session-id existing-session-uuid
truenorth run --interactive    # Start a REPL session

# Start the web server
truenorth serve
truenorth serve --port 9090
truenorth serve --host 0.0.0.0 --port 8080

# Resume a paused session
truenorth resume <session-uuid>

# Skill management
truenorth skill list
truenorth skill install <path-or-url>
truenorth skill remove <skill-name>
truenorth skill validate <path>

# Memory management
truenorth memory query --query "search term" --scope project
truenorth memory consolidate
truenorth memory export --format json

# Configuration
truenorth config show
truenorth config validate

# Version info
truenorth version
truenorth --format json version
```

---

## 2. Docker Deployment

### Build and Run

```bash
# Build the Docker image
docker build -t truenorth:latest .

# Run with environment variables
docker run -d \
  --name truenorth \
  -p 8080:8080 \
  -v truenorth-data:/data/truenorth \
  -e ANTHROPIC_API_KEY=sk-ant-... \
  -e TRUENORTH_AUTH_TOKEN=your-secret-token \
  truenorth:latest

# View logs
docker logs -f truenorth
```

### Docker Compose (Recommended for Local Docker)

The included `docker-compose.yml` is the simplest path to a running TrueNorth instance:

```bash
# Copy and configure
cp .env.example .env
# Edit .env: set ANTHROPIC_API_KEY and/or OPENAI_API_KEY

cp config.toml.example config.toml
# Edit config.toml as needed

# Start
docker compose up -d

# View logs
docker compose logs -f truenorth

# Stop
docker compose down

# Stop and remove data volume (CAUTION: deletes all memory and sessions)
docker compose down -v
```

The `docker-compose.yml` mounts:
- `truenorth-data` named volume → `/data/truenorth` (persistent data)
- `./config.toml` → `/data/truenorth/config.toml` (read-only config)

### Dockerfile Architecture

The Dockerfile uses a two-stage build to minimize the final image size:

**Stage 1 (Builder)**: `rust:1.80-bookworm`
- Copies `Cargo.toml` and `Cargo.lock` first to cache dependencies
- Uses a placeholder `lib.rs` trick to pre-build all dependencies before copying source
- Builds the final `truenorth-cli` binary with `cargo build --release`
- Result: a fully statically-linked binary (SQLite is bundled via rusqlite's `bundled` feature)

**Stage 2 (Runtime)**: `debian:bookworm-slim`
- Copies only the binary from Stage 1
- Creates a non-root `truenorth` user for security
- Installs only `ca-certificates` (needed for HTTPS API calls)
- Final image size: ~50–60 MB

### Image Customization

To add custom skills to the image:

```dockerfile
# Extend the official image
FROM truenorth:latest

# Copy skill files into the container's data directory
COPY my-skills/ /data/truenorth/skills/
```

To use a custom `config.toml` baked into the image:

```dockerfile
FROM truenorth:latest
COPY production-config.toml /data/truenorth/config.toml
```

---

## 3. Fly.io Deployment

[Fly.io](https://fly.io) is the recommended managed hosting platform for TrueNorth. It provides persistent volumes for data storage, automatic HTTPS, and global edge deployment.

### Prerequisites

```bash
# Install the Fly CLI
curl -L https://fly.io/install.sh | sh

# Authenticate
fly auth login
```

### First Deployment

```bash
# From the truenorth repository root
# The fly.toml is already configured — just deploy
fly deploy

# Set your API keys as secrets (not environment variables — they're encrypted)
fly secrets set ANTHROPIC_API_KEY=sk-ant-...
fly secrets set TRUENORTH_AUTH_TOKEN=your-secret-token

# Optional: set additional provider keys
fly secrets set OPENAI_API_KEY=sk-...
fly secrets set GOOGLE_AI_API_KEY=AIza...
```

### fly.toml Explained

```toml
app = "truenorth"          # Your app name on Fly.io (must be globally unique)
primary_region = "iad"     # Primary deployment region (iad = US East)

[build]
  dockerfile = "Dockerfile" # Uses the multi-stage Dockerfile

[env]
  TRUENORTH_LOG_LEVEL = "info"
  TRUENORTH_DATA_DIR = "/data/truenorth"    # Matches the volume mount

[http_service]
  internal_port = 8080        # Port the TrueNorth binary listens on
  force_https = true          # Redirect all HTTP to HTTPS automatically
  auto_stop_machines = "stop" # Stop idle machines to save cost
  auto_start_machines = true  # Auto-start on incoming requests
  min_machines_running = 0    # Allow scaling to zero (cost optimization)

  [http_service.concurrency]
    type = "requests"
    hard_limit = 250          # Reject requests above this (prevents overload)
    soft_limit = 200          # Begin load-balancing above this

[[vm]]
  size = "shared-cpu-2x"     # 2 shared vCPUs, 512 MB RAM
  memory = "1gb"             # Increase to "2gb" for heavy workloads

[mounts]
  source = "truenorth_data"      # Named volume (created automatically)
  destination = "/data/truenorth"  # Mount point inside the container
```

### Volume Management

Fly.io persistent volumes store all TrueNorth data (SQLite databases, memory files, sessions):

```bash
# List volumes
fly volumes list

# Create a volume in a specific region (if not auto-created)
fly volumes create truenorth_data --region iad --size 10  # 10 GB

# Extend a volume (can only grow, not shrink)
fly volumes extend <volume-id> --size 20
```

### Scaling

```bash
# Scale to multiple regions
fly scale count 2 --region iad --region lhr

# Change machine size (memory-intensive for large models)
fly scale vm performance-2x

# View running machines
fly status
```

### Monitoring on Fly.io

```bash
# Live logs
fly logs

# SSH into a running machine for debugging
fly ssh console

# Inspect metrics in the Fly.io dashboard
fly dashboard
```

---

## 4. Railway Deployment

[Railway](https://railway.app) is an alternative deployment platform with a simpler setup experience.

### Deploy from GitHub

1. Go to [railway.app/new](https://railway.app/new) and select "Deploy from GitHub repo"
2. Select your fork of the TrueNorth repository
3. Railway auto-detects the `Dockerfile` and configures the build

### Environment Variables

In the Railway project settings, add the following variables:

| Variable | Value |
|----------|-------|
| `ANTHROPIC_API_KEY` | `sk-ant-...` |
| `TRUENORTH_AUTH_TOKEN` | `your-secret-token` |
| `TRUENORTH_DATA_DIR` | `/data/truenorth` |
| `TRUENORTH_LOG_LEVEL` | `info` |
| `PORT` | `8080` |

### Persistent Storage

Railway provides persistent volumes via the "Volume" service. Add a volume:

1. In your Railway project, click **+ New** → **Volume**
2. Set the mount path to `/data/truenorth`
3. Choose a size (start with 10 GB)

### `railway.toml` (Optional)

Create a `railway.toml` for Railway-specific configuration:

```toml
[build]
builder = "DOCKERFILE"
dockerfilePath = "Dockerfile"

[deploy]
startCommand = "truenorth serve --host 0.0.0.0 --port $PORT"
restartPolicyType = "ON_FAILURE"
restartPolicyMaxRetries = 3
```

---

## 5. Environment Variables Reference

All environment variables can override configuration file values. Variables prefixed with `TRUENORTH_` are specific to TrueNorth. Provider API key variables follow each provider's convention.

### Core Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TRUENORTH_DATA_DIR` | `~/.truenorth` | Root directory for all TrueNorth data |
| `TRUENORTH_LOG_LEVEL` | `info` | Log verbosity: `error`, `warn`, `info`, `debug`, `trace` |
| `TRUENORTH_LOG_FORMAT` | `text` | Log format: `text` (human-readable) or `json` |
| `TRUENORTH_AUTH_TOKEN` | *(unset)* | Bearer token for API authentication. If unset, auth is disabled |
| `RUST_LOG` | *(unset)* | Overrides `TRUENORTH_LOG_LEVEL` with fine-grained module filters |

### LLM Provider API Keys

| Variable | Provider | Notes |
|----------|---------|-------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) | Format: `sk-ant-api03-...` |
| `OPENAI_API_KEY` | OpenAI (GPT-4, o-series) | Format: `sk-proj-...` |
| `GOOGLE_AI_API_KEY` | Google (Gemini) | From [AI Studio](https://aistudio.google.com/) |
| `GROQ_API_KEY` | Groq (via openai_compat) | For ultra-fast inference |

No API key is needed for Ollama (local inference) — configure `base_url` in `config.toml`.

### Advanced Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TRUENORTH_SKILLS_DIR` | `<data_dir>/skills` | Override the skills directory path |
| `TRUENORTH_MODELS_DIR` | `<data_dir>/models` | Override the embedding model cache path |
| `TRUENORTH_MAX_STEPS` | `50` | Override `max_steps_per_task` |
| `TRUENORTH_COMPACT_THRESHOLD` | `0.70` | Context compaction threshold (0.0–1.0) |

---

## 6. Configuration File Reference

TrueNorth loads `config.toml` from the data directory (`~/.truenorth/config.toml` by default, or the path specified by `--config` on the CLI).

### Minimal Configuration

```toml
# config.toml — minimal working configuration

[llm]
primary = "anthropic"
fallback_order = ["openai"]

[[providers]]
name = "anthropic"
model = "claude-opus-4-5"
api_key_env = "ANTHROPIC_API_KEY"

[[providers]]
name = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"
```

### Complete Configuration Reference

```toml
# config.toml — all available options with defaults

# ─── LLM Routing ────────────────────────────────────────────────────────────

[llm]
# The name of the primary LLM provider (must match a [[providers]] entry).
primary = "anthropic"

# Fallback providers in order. Tried if primary fails.
fallback_order = ["openai", "ollama"]

# Default context window size in tokens.
default_context_size = 200000

# Default maximum output tokens per completion.
default_max_tokens = 8192

# Default sampling temperature (0.0 = deterministic, 1.0 = very random).
default_temperature = 0.7

# Enable extended thinking / chain-of-thought for supported providers.
enable_thinking = false

# Token budget for extended thinking.
thinking_budget = 10000

# ─── Provider Configurations ────────────────────────────────────────────────

[[providers]]
name = "anthropic"
model = "claude-opus-4-5"
api_key_env = "ANTHROPIC_API_KEY"   # environment variable name for the key
enabled = true

[[providers]]
name = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"
enabled = true

[[providers]]
name = "google"
model = "gemini-2.0-flash"
api_key_env = "GOOGLE_AI_API_KEY"
enabled = false  # disabled until key is set

[[providers]]
name = "ollama"
model = "llama3.2"
base_url = "http://localhost:11434"  # Ollama server URL
enabled = false                      # disabled unless Ollama is running

# OpenAI-compatible endpoint (e.g., Groq, LM Studio, Together)
[[providers]]
name = "groq"
model = "llama-3.3-70b-versatile"
base_url = "https://api.groq.com/openai/v1"
api_key_env = "GROQ_API_KEY"
enabled = false

# ─── Memory System ───────────────────────────────────────────────────────────

[memory]
# Enable semantic (embedding-based) search and deduplication.
enable_semantic_search = true

# Embedding provider: "local" (fastembed, no API key) or "openai".
embedding_provider = "local"

# Maximum search results per query.
max_search_results = 10

# Cosine similarity threshold for semantic deduplication (0.0–1.0).
# Entries above this similarity are considered duplicates and merged.
deduplication_threshold = 0.85

# Context compaction trigger threshold (0.0–1.0).
# When context token usage exceeds this fraction, compaction is triggered.
compact_threshold = 0.70

# Handoff threshold (0.0–1.0).
# When usage exceeds this, a new context window is started.
handoff_threshold = 0.90

# Halt threshold (0.0–1.0).
# When usage exceeds this, execution is paused and state is saved.
halt_threshold = 0.98

# Automatically consolidate memory after sessions end.
auto_consolidate = true

# Directory for local embedding model cache.
model_cache_dir = "~/.truenorth/models"

# ─── WASM Sandbox ────────────────────────────────────────────────────────────

[sandbox]
# Enable the WASM sandbox for third-party tools.
# Set to false only for development (disables all sandboxing).
enabled = true

# Maximum memory per WASM instance (bytes). Default: 64 MiB.
max_memory_bytes = 67108864

# CPU fuel units per WASM execution (~10M simple operations).
max_fuel = 10000000

# Wall-clock timeout per WASM execution (milliseconds).
max_execution_ms = 30000

# Allow WASM modules to access the system clock.
allow_clock = true

# Allow WASM modules to generate random numbers.
allow_random = true

# ─── System Settings ─────────────────────────────────────────────────────────

# Root directory for all TrueNorth data.
data_dir = "~/.truenorth"

# Directory for installed skill files.
skills_dir = "~/.truenorth/skills"

# Workspace directory (project files).
workspace_dir = "."

# Log verbosity level.
log_level = "info"

# Enable the web UI server.
enable_web_ui = true

# Port for the web UI server.
web_ui_port = 3000

# Maximum steps per task (loop guard).
max_steps_per_task = 50

# Maximum LLM routing loops before exhausted.
max_routing_loops = 2

# Require human approval of execution plans before proceeding.
require_plan_approval = false

# Enable the negative checklist verifier.
enable_negative_checklist = true
```

---

## 7. TLS and Reverse Proxy Setup

TrueNorth's built-in server (`truenorth serve`) does not handle TLS directly. For production deployments that are not on Fly.io or Railway (which handle TLS automatically), use a reverse proxy.

### Nginx

```nginx
# /etc/nginx/sites-available/truenorth

server {
    listen 80;
    server_name truenorth.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    server_name truenorth.example.com;

    ssl_certificate /etc/letsencrypt/live/truenorth.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/truenorth.example.com/privkey.pem;

    # Proxy to TrueNorth
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;

        # Required for WebSocket upgrade
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";

        # Required headers
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Timeout for long-running LLM requests
        proxy_read_timeout 300s;
        proxy_send_timeout 300s;

        # SSE: disable buffering for streaming responses
        proxy_buffering off;
        proxy_cache off;
    }
}
```

Enable with:
```bash
ln -s /etc/nginx/sites-available/truenorth /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx
```

### Caddy (Automatic TLS)

[Caddy](https://caddyserver.com) automatically handles TLS via Let's Encrypt:

```caddyfile
# /etc/caddy/Caddyfile

truenorth.example.com {
    reverse_proxy 127.0.0.1:8080 {
        # Required for WebSocket and SSE
        transport http {
            read_buffer 0
        }

        # Flush immediately for SSE
        flush_interval -1
    }
}
```

### Cloudflare Tunnel (No Open Ports)

For deployments where you cannot open inbound ports:

```bash
# Install cloudflared
# https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/

cloudflared tunnel login
cloudflared tunnel create truenorth
cloudflared tunnel route dns truenorth truenorth.example.com

# Run the tunnel (or use a systemd service)
cloudflared tunnel run --url http://localhost:8080 truenorth
```

---

## 8. Monitoring and Health Checks

### Health Check Endpoint

TrueNorth exposes a health check at `/health` (no authentication required):

```bash
curl http://localhost:8080/health
```

Response:
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_secs": 3600
}
```

Use this for:
- Container health checks (`HEALTHCHECK` in Dockerfile)
- Load balancer health probes
- Uptime monitoring services

### Docker Health Check

Add to your Dockerfile or `docker-compose.yml`:

```yaml
# docker-compose.yml
services:
  truenorth:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 15s
```

### Structured Logging for Aggregation

Enable JSON logging for integration with log aggregation tools (Datadog, Grafana Loki, CloudWatch):

```bash
TRUENORTH_LOG_FORMAT=json RUST_LOG=info truenorth serve
```

JSON log format:
```json
{
  "timestamp": "2026-03-31T22:00:00Z",
  "level": "INFO",
  "target": "truenorth_llm::router",
  "fields": {
    "provider": "anthropic",
    "model": "claude-opus-4-5",
    "prompt_tokens": 1234,
    "completion_tokens": 567,
    "latency_ms": 2341
  },
  "span": {
    "name": "llm_complete",
    "session_id": "uuid"
  }
}
```

### Metrics (Planned)

Prometheus metrics endpoint is planned for a future release at `/metrics`. Key metrics will include:
- `truenorth_llm_requests_total{provider, model, status}`
- `truenorth_llm_latency_seconds{provider, model}`
- `truenorth_memory_entries_total{scope}`
- `truenorth_sessions_active`
- `truenorth_tool_calls_total{tool, status}`

### Alerting Recommendations

Set up alerts for:

| Condition | Alert |
|-----------|-------|
| `/health` returns non-200 | Service down |
| All LLM providers failing (`AllProvidersExhausted` in logs) | API key rotation needed |
| Disk usage > 80% | Volume expansion needed |
| Memory (RAM) > 85% | Machine upsize needed |
| LLM latency p99 > 30s | Provider degradation |

---

## 9. Backup and Migration

### What Needs to Be Backed Up

All persistent state lives in the data directory (`TRUENORTH_DATA_DIR`, default `~/.truenorth/`):

| Path | Contents | Criticality |
|------|---------|------------|
| `memory/project.db` | Project-scoped memory (SQLite) | High |
| `memory/identity.db` | User identity and preferences | High |
| `memory/vault/` | Obsidian-compatible Markdown files | High |
| `sessions/` | Paused session state | Medium |
| `memory/tantivy_index/` | Search index (rebuildable) | Low — can be rebuilt |
| `skills/` | Installed skill files | Low — reinstallable |
| `models/` | Embedding model cache | Low — re-downloadable |
| `config.toml` | Configuration | Medium — keep in version control |

### Backup Strategy

**Minimum viable backup** — the SQLite databases and vault:

```bash
#!/bin/bash
# backup.sh — run daily via cron

DATA_DIR="${TRUENORTH_DATA_DIR:-$HOME/.truenorth}"
BACKUP_DIR="/backup/truenorth/$(date +%Y%m%d)"

mkdir -p "$BACKUP_DIR"

# Backup SQLite databases using sqlite3's online backup
sqlite3 "$DATA_DIR/memory/project.db" ".backup '$BACKUP_DIR/project.db'"
sqlite3 "$DATA_DIR/memory/identity.db" ".backup '$BACKUP_DIR/identity.db'"

# Backup vault and skills (simple file copy)
cp -r "$DATA_DIR/memory/vault" "$BACKUP_DIR/vault"
cp -r "$DATA_DIR/skills" "$BACKUP_DIR/skills"
cp "$DATA_DIR/config.toml" "$BACKUP_DIR/config.toml" 2>/dev/null || true

echo "Backup complete: $BACKUP_DIR"
```

**Docker volume backup**:

```bash
# Backup the Docker named volume
docker run --rm \
  -v truenorth-data:/data \
  -v $(pwd):/backup \
  debian:bookworm-slim \
  tar czf /backup/truenorth-backup-$(date +%Y%m%d).tar.gz /data
```

**Fly.io volume backup**:

```bash
# SSH into the machine and tar the data directory
fly ssh console -C "tar czf /tmp/backup.tar.gz /data/truenorth"

# Copy the backup to local machine
fly sftp get /tmp/backup.tar.gz ./truenorth-backup-$(date +%Y%m%d).tar.gz
```

### Restore from Backup

```bash
# Stop TrueNorth
systemctl stop truenorth  # or docker compose down

# Restore SQLite databases
cp backup/project.db "$DATA_DIR/memory/project.db"
cp backup/identity.db "$DATA_DIR/memory/identity.db"

# Restore vault and skills
cp -r backup/vault "$DATA_DIR/memory/vault"
cp -r backup/skills "$DATA_DIR/skills"

# Rebuild Tantivy index (automatic on next startup — or force it)
rm -rf "$DATA_DIR/memory/tantivy_index/"

# Restart TrueNorth
systemctl start truenorth  # or docker compose up -d
```

### Migration Between Versions

When upgrading TrueNorth, check the release notes for schema migrations.

**SQLite schema migrations** are applied automatically at startup. When the schema changes, TrueNorth:
1. Opens the database
2. Reads the current `schema_version` from the `schema_version` table
3. Applies any pending migrations in order
4. Updates `schema_version`

If a migration fails, TrueNorth halts with an error message. Restore from backup, then file a bug report.

**Session state migrations**: Session state files contain a `schema_version` field. If the running binary cannot migrate a session file (because it's too old), it will refuse to resume that session rather than potentially corrupting state. The session will remain on disk and can be accessed after downgrading.

### Data Export

Export memory data for analysis or migration to another system:

```bash
# Export all project memory as JSON
truenorth memory export --scope project --format json > project-memory.json

# Export as Markdown (same as vault sync)
truenorth memory export --scope project --format markdown --output ./export/

# Export session history
truenorth session export <session-uuid> --format json > session.json
```

---

*For architecture details, see [ARCHITECTURE.md](ARCHITECTURE.md). For developer setup, see [DEVELOPMENT.md](DEVELOPMENT.md).*
