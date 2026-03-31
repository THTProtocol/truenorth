# Contributing to TrueNorth

Thank you for your interest in contributing to TrueNorth.

## Development Setup

1. Install Rust 1.80+ via [rustup](https://rustup.rs/)
2. Clone the repository
3. Run `cargo build --workspace` to verify your setup
4. Run `cargo test --workspace` to ensure all tests pass

## Architecture

TrueNorth follows a strict crate-based architecture. Before making changes, understand which crate owns the functionality you're modifying:

- **truenorth-core**: Types and traits ONLY. No implementations. Changes here affect all crates.
- **truenorth-llm**: LLM provider implementations. Add new providers here.
- **truenorth-memory**: Memory tier implementations. Modify storage/search here.
- **truenorth-tools**: Tool implementations. Add built-in tools here.
- **truenorth-skills**: Skill loading and parsing. Modify SKILL.md format here.
- **truenorth-visual**: Visual reasoning events. Modify rendering here.
- **truenorth-orchestrator**: Agent loop logic. Modify execution strategies here.
- **truenorth-web**: HTTP server and frontend. Modify API endpoints here.
- **truenorth-cli**: CLI commands. Add new commands here.

## Pull Request Checklist

- [ ] `cargo build --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace` passes with no warnings
- [ ] `cargo fmt --all -- --check` passes
- [ ] All NEGATIVE_CHECKLIST items verified
- [ ] Documentation updated if public API changed
- [ ] Tests added for new functionality

## Code Style

- All public items must have doc comments
- Use `thiserror` for error types, `anyhow` for error propagation
- Use `tracing` for logging (not `println!` or `log`)
- Async functions use `tokio`; never block the runtime
- Use `parking_lot` for sync locks when lock is not held across await points

## Commit Messages

Use conventional commits:
- `feat(crate): description` for new features
- `fix(crate): description` for bug fixes
- `docs(crate): description` for documentation
- `test(crate): description` for tests
- `refactor(crate): description` for refactoring
