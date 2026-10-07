# Contributing to RDPBeacon

Thank you for your interest in contributing to **RDPBeacon**! We welcome bug reports, documentation improvements, feature suggestions, and code contributions.

---

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/) (stable toolchain, 2021 edition)
- Git

### Cloning and Building

```bash
git clone https://github.com/hclhls/RDPBeacon.git
cd RDPBeacon
cargo build
```

---

## Development Workflow

### 1. Code Checks

Ensure your changes compile cleanly without warnings:

```bash
# Check compilation across default profile
cargo check

# Run tests
cargo test

# Build optimized release binary
cargo build --release
```

### 2. Testing Locally

You can test changes against your local environment safely:

```bash
# Inspect environment and verify window detection
cargo run -- check

# Run a single beacon cycle with verbose logging
cargo run -- -v once

# Run daemon with a temporary short interval for quick testing
cargo run -- --config <(printf 'interval = "10s"\nidle_threshold = "5s"\njitter = "2s"\n') -v run
```

---

## Code Architecture

RDPBeacon is organized into decoupled modules:

- **[`src/backend/`](src/backend/)**:
  - `Backend` trait: defines idle detection, window lookup, focus switching, and beacon injection.
  - Platform implementations:
    - [`x11.rs`](src/backend/x11.rs): Linux X11 implementation using pure-Rust `x11rb`.
    - [`wayland.rs`](src/backend/wayland.rs): Linux Wayland implementation using `/dev/uinput` via `evdev`.
    - [`windows.rs`](src/backend/windows.rs): Windows Win32 implementation using the `windows` crate.
  - Window scoring & classification (`mod.rs`): prioritizes active remote session windows over client launcher dialogs and toolbars.
- **[`src/config.rs`](src/config.rs)**: Strongly typed configuration with duration parsing (`humantime-serde`).
- **[`src/scheduler.rs`](src/scheduler.rs)**: Daemon scheduling loop, idle threshold evaluation, and focus management.
- **[`src/main.rs`](src/main.rs)**: CLI argument parsing and logger initialization.

---

## Submitting Pull Requests

1. **Fork the repository** and create a feature branch (`git checkout -b feat/your-feature`).
2. **Write tests** covering your new functionality or bug fix.
3. **Verify tests pass**:
   ```bash
   cargo test
   ```
4. **Use Conventional Commits**:
   - `feat: add support for ...`
   - `fix: resolve issue with ...`
   - `docs: update instructions for ...`
   - `test: add unit tests for ...`
   - `refactor: clean up ...`
5. **Open a Pull Request** against the `main` branch with a clear description of your change and testing steps.
