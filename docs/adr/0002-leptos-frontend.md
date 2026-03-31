# ADR-0002: Leptos for Frontend

## Status: Accepted

## Context

The web frontend needs a reactive UI framework. Options: React (via Trunk/wasm-pack), Yew, Dioxus, or Leptos.

## Decision

Use Leptos with server-side rendering via Axum integration.

## Rationale

1. **Same language as backend** — No context switching, shared types between frontend and backend
2. **Fine-grained reactivity** — Leptos signals are more efficient than virtual DOM diffing
3. **Axum integration** — `leptos_axum` provides seamless SSR with our existing Axum server
4. **Growing ecosystem** — Active development, good documentation, production users

## Consequences

- Smaller ecosystem than React
- Fewer UI component libraries available
- Developers need to learn Leptos-specific patterns
