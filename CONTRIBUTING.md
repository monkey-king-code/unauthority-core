# Contributing to Unauthority (LOS)

Thank you for your interest in contributing to Unauthority. This document provides guidelines and standards for contributing to the project.

---

## Table of Contents

1. [Code of Conduct](#code-of-conduct)
2. [Getting Started](#getting-started)
3. [Development Setup](#development-setup)
4. [Code Standards](#code-standards)
5. [Pull Request Process](#pull-request-process)
6. [Testing Requirements](#testing-requirements)
7. [Commit Guidelines](#commit-guidelines)
8. [Architecture Overview](#architecture-overview)
9. [Security](#security)

---

## Code of Conduct

This project follows a simple principle: **be respectful and constructive**. All contributors are expected to:
- Treat others with respect
- Provide constructive feedback
- Focus on the technical merits of contributions
- Accept decisions gracefully

---

## Getting Started

### Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| **Rust** | 1.75+ (2021 edition) | Backend, node, crypto |
| **Flutter** | 3.27+ | Wallet and validator dashboard |
| **Tor** | Latest stable | Network transport |
| **Git** | Latest stable | Version control |

### Fork and Clone

```bash
git clone https://github.com/monkey-king-code/unauthority-core.git
cd unauthority-core
```

---

## Development Setup

### Build the Node

```bash
# Testnet build (default — includes faucet, relaxed validation)
cargo build --release -p los-node

# Mainnet build (strict — no faucet, enforced signing)
cargo build --release -p los-node --features mainnet
```

### Run Tests

```bash
# All tests
cargo test --release --workspace --all-features

# Specific crate
cargo test --release -p los-core
cargo test --release -p los-consensus

# With output
cargo test --release -p los-core -- --nocapture
```

### Run Linter

```bash
# Zero warnings enforced in CI
cargo clippy --workspace --all-features -- -D warnings
```

### Run Formatter

```bash
cargo fmt --all --check   # Check only
cargo fmt --all           # Auto-format
```

### Build Flutter Apps

```bash
# Wallet
cd flutter_wallet
flutter pub get
flutter test
cd ..

# Validator
cd flutter_validator
flutter pub get
flutter test
cd ..
```

---

## Code Standards

### Rust

- **Edition:** 2021
- **Error handling:** Use `Result<T, E>` and `Option<T>` — no `unwrap()` in production code
- **Panics:** Zero `panic!()`, `todo!()`, `unimplemented!()` in mainnet-reachable paths
- **Floating-point:** Absolutely no `f32`/`f64` in consensus or financial logic — use `u128` integer arithmetic
- **Math safety:** Use `checked_add`, `checked_mul`, `checked_sub`, `checked_div` for all arithmetic
- **Naming:** `snake_case` for functions/variables, `PascalCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants
- **Documentation:** All public functions must have `///` doc comments
- **Tests:** Every module should have `#[cfg(test)]` tests

### Dart/Flutter

- **Null safety:** Use sound null safety throughout
- **Naming:** `camelCase` for variables/functions, `PascalCase` for classes
- **Linting:** Follow `analysis_options.yaml` rules
- **State management:** Use `setState` and `ChangeNotifier` patterns

### Comments

- Write comments that explain **why**, not **what**
- Keep comments up-to-date when code changes
- Use `TODO(username):` format for temporary notes (not allowed in mainnet releases)
- Document all public APIs with examples

---

## Pull Request Process

### Before Submitting

1. **Create a branch** from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Write tests** for new functionality

3. **Run the full test suite:**
   ```bash
   cargo test --release --workspace --all-features
   cargo clippy --workspace --all-features -- -D warnings
   ```

4. **Ensure zero warnings** — CI enforces `cargo clippy -D warnings`

### PR Requirements

- [ ] All tests pass
- [ ] Zero clippy warnings
- [ ] Code formatted with `cargo fmt`
- [ ] New public APIs have doc comments
- [ ] Commit messages follow [conventional commits](#commit-guidelines)
- [ ] No `unwrap()`, `todo!()`, or `f32`/`f64` in consensus paths

### Review Process

1. Submit PR against `main` branch
2. Automated CI runs (7 jobs: lint, test, build, format, Flutter tests)
3. Code review by maintainer
4. Address feedback and push fixes
5. Squash merge once approved

---

## Testing Requirements

### Unit Tests

Every module should have tests. Example:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isqrt_basic() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(100), 10);
    }
}
```

### Integration Tests

Integration tests live in the `tests/` directory and test cross-crate functionality.

### Test Coverage

- All financial/consensus logic must have tests
- Edge cases: zero values, overflow, max values
- Error paths must be tested

---

## Commit Guidelines

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): description

[optional body]
```

### Types

| Type | Purpose |
|---|---|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation changes |
| `refactor` | Code restructuring (no behavior change) |
| `test` | Adding or updating tests |
| `chore` | Build config, CI, tooling |
| `perf` | Performance improvement |

### Examples

```
feat(core): add PoW mining distribution with SHA3-256
fix(consensus): prevent double-counting votes during view change
docs(api): add Exchange Integration guide with code examples
refactor(network): extract Tor connection logic into separate module
test(crypto): add edge case tests for Dilithium5 key derivation
```

---

## Architecture Overview

See [ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full system design. Key crates:

| Crate | Purpose |
|---|---|
| `los-node` | Main validator binary |
| `los-core` | Blockchain primitives (Block, Ledger, Oracle) |
| `los-consensus` | aBFT consensus, slashing, checkpoints |
| `los-network` | P2P, Tor transport, fee scaling |
| `los-crypto` | Dilithium5 and SHA-3 cryptography |
| `los-vm` | WASM smart contract engine |
| `los-contracts` | USP-01 token and DEX AMM contracts |
| `los-cli` | Command-line interface |
| `los-sdk` | External integration SDK |

---

## Security

If you discover a security vulnerability, **DO NOT** open a public issue. See [SECURITY.md](SECURITY.md) for responsible disclosure instructions.

---

## License

By contributing, you agree that your contributions will be licensed under the [AGPL-3.0 License](LICENSE).
