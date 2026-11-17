# Development

Use Rust 1.95.0, edition 2024, and ordinary Cargo commands:

```sh
cargo run -- /path/to/repository
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

Keep `Cargo.lock` checked in. Prioritize clean design, DRY, KISS, and YAGNI.
Comments should explain a non-obvious constraint, briefly, rather than narrate code.

# Boundaries

- `git`: repository discovery, machine-readable status, and read-only Git subprocesses.
- `diff`: patch parsing, line alignment, and intraline highlighting; no terminal or Git I/O.
- `app`: navigation, selection, search, and refresh state; no Git commands or rendering.
- `worker`: one background refresh at a time, using threads and channels.
- `ui`: Ratatui rendering; no repository operations.
- `main`: CLI, terminal lifetime, events, and refresh scheduling.

Never stage, edit, or otherwise mutate the repository being viewed. Disable optional
Git locks, external diff programs, text conversion, and filesystem monitor hooks.
Preserve paths as paths, parse NUL-delimited status, and reject outdated worker results.

# Validation

Prefer behavior-focused smoke tests using temporary repositories and headless rendering.
Cover final behavior, not logging or incidental implementation details. Run formatting,
Clippy, and tests before delivery. Distinguish automated PTY checks from human testing
in WezTerm/Zellij. If adding logs, include operation and relevant path context, use
consistent levels, and avoid per-frame or per-line noise.
