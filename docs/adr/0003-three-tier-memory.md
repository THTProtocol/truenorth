# ADR-0003: Three-Tier Memory Architecture

## Status: Accepted

## Context

AI agents need persistent memory across sessions. Most frameworks use a single vector store. We need something more structured.

## Decision

Implement three memory tiers with automatic promotion:
1. **Session** — Current conversation context (volatile, compacted)
2. **Project** — Cross-session knowledge for the active project (SQLite + Tantivy)
3. **Identity** — Long-term user preferences and behavioral patterns (Obsidian-synced)

## Rationale

1. **Cognitive fidelity** — Mirrors how humans organize knowledge (working memory → long-term → personality)
2. **Obsidian compatibility** — Users can browse, edit, and extend their agent's memory as Markdown files
3. **Search quality** — Hybrid search (BM25 + semantic) outperforms pure vector similarity
4. **Privacy** — All data stays local. No cloud vector DB dependency.

## Consequences

- More complex than a single vector store
- Consolidation scheduler needed for tier promotion
- Obsidian sync requires file watcher (notify crate)
