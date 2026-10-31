# Agent Guidelines for rblue

This document provides guidelines for maintaining high-quality Rust code in the
rblue workspace. These rules MUST be followed by all AI coding agents and
contributors.

## Project Overview

rblue is a cross-platform async Bluetooth Low Energy (BLE) library: one unified
Rust API across Windows (WinRT), macOS/iOS (CoreBluetooth), Linux (BlueZ),
Android (JNI), Web (Web Bluetooth via web-sys), and HarmonyOS (ArkTS driver
bridged over Node-API). Scope is BLE Central (GATT client) first; GATT server
and advertising are Phase 2.

The detailed architecture and implementation plan live one directory above this
repository (`rblue-implementation-plan.md`). Consult it before making
architectural decisions; this file governs day-to-day code quality.

## Project Structure: Where Code Goes

```
rblue/
├── Cargo.toml                    # [workspace]
├── xtask/                        # build/packaging orchestration (cargo xtask <cmd>)
├── crates/
│   ├── rblue-api/                # core traits, error model, capability negotiation
│   ├── rblue-mock/               # in-memory mock backend (tests/prototyping)
│   ├── rblue/                    # aggregate crate: feature-selects backends
│   ├── rblue-winrt/              # Windows backend (shell over btleplug fork)
│   ├── rblue-corebluetooth/      # macOS/iOS backend (shell)
│   ├── rblue-bluez/              # Linux backend (shell)
│   ├── rblue-android/            # Android backend (shell over btleplug JNI)
│   ├── rblue-web/                # wasm32 WebBluetooth backend (web-sys)
│   ├── rblue-ohos/               # HarmonyOS: Rust side of the BleDriver protocol
│   ├── rblue-driver/             # BleDriver protocol definition (Driver-track backends)
│   ├── rblue-ffi/                # UniFFI facade (Kotlin/Swift)
│   └── rblue-napi/               # ohos-rs facade (ArkTS)
└── bindings/                     # kotlin/ · swift/ · ts/ · arkts/ packaging projects
```

Hard boundaries:

- `rblue-api` MUST stay pure Rust: no platform crates, no async runtime
  dependency, no FFI dependency, `#![no_std]`-friendly where practical.
- Platform-specific code lives ONLY in its backend crate or behind
  target-gated features in the aggregate `rblue` crate (same pattern as
  `btleplug::platform`).
- Backend shells over the btleplug fork MUST stay thin adapters
  (`btleplug Central/Peripheral → rblue-api`). Never fix a bug by editing
  forked btleplug internals in ways that would break a future `git rebase`
  against upstream; adapt in the shell instead.
- The Driver track (HarmonyOS, optionally Android later) injects a
  host-language implementation of the `BleDriver` protocol; the protocol shape
  is defined once in `rblue-driver` and MUST NOT be forked per platform.

## Your Core Principles

All code you write MUST be fully optimized.

"Fully optimized" includes:

- maximizing algorithmic big-O efficiency for memory and runtime
- following proper style conventions for Rust (e.g. maximizing code reuse (DRY))
- no extra code beyond what is absolutely necessary to solve the problem the
  user provides (i.e. no technical debt)
  - If a crate can be imported to significantly reduce the amount of new code
    required to implement a function at optimal performance, and the crate
    itself is small and does not have much overhead, ALWAYS use the crate
    instead — subject to the dependency portability rule below.

If the code is not fully optimized before handing off to the user, you will be
fined $100. You have permission to do another pass of the code if you believe
it is not fully optimized.

## Preferred Tools

- Use `cargo` for project management, building, and dependency management.
  Build orchestration belongs in `xtask`, not shell scripts.
- Use `tokio` as the async runtime in tests, examples, and binaries. `rblue-api`
  itself MUST remain runtime-agnostic.
- Use `thiserror` for the error model in library crates. Use `anyhow` ONLY in
  binaries, examples, and test glue — NEVER as part of a public library API.
- Use `tracing` for logging and diagnostics. When reporting errors to the
  console, use `tracing::error!` or `log::error!` instead of `println!`.
- Use `serde` with `serde_json` only where JSON is genuinely needed (e.g. mock
  device scripting, test fixtures). Do not pull it into `rblue-api`.
- Platform binding crates (`windows`, `objc2`, `bluer`, `jni`, `web-sys`,
  `napi`/`ohos-rs`, `uniffi`) are confined to the crate that owns that
  platform or facade.
- Use `indicatif` to track long-running operations in `xtask` with progress
  bars. The message should be contextually sensitive.

### Dependency Portability

Every dependency must compile on all supported targets (desktop, `aarch64-apple-ios`,
`wasm32-unknown-unknown`, OHOS). Before adding a dependency:

- **MUST** verify it works on `wasm32` or gate it behind a target-specific
  feature/crate boundary.
- Prefer crates already used by btleplug/bluer for platform work — they are
  known-portable.
- Check MSRV compatibility before adding or upgrading.

## Code Style and Formatting

- **MUST** use meaningful, descriptive variable and function names
- **MUST** follow Rust API Guidelines and idiomatic Rust conventions
- **MUST** use 4 spaces for indentation (never tabs)
- **NEVER** use emoji, or unicode that emulates emoji (e.g. ✓, ✗), in code or
  comments. The only exception is when writing tests and testing the impact of
  multibyte characters.
- Use snake_case for functions/variables/modules, PascalCase for types/traits,
  SCREAMING_SNAKE_CASE for constants
- Limit line length to 100 characters (rustfmt default)
- **MUST** avoid including redundant comments which are tautological or
  self-demonstrating (e.g. cases where it is easily parsable what the code does
  at a glance or its function name giving sufficient information as to what the
  code does, so the comment does nothing other than waste user time)
- **MUST** avoid including comments which leak what this file contains, or leak
  the original user prompt, ESPECIALLY if it's irrelevant to the output code.

## Documentation

- **MUST** include doc comments for all public functions, structs, enums, and
  methods
- **MUST** document function parameters, return values, and errors
- Keep comments up-to-date with code changes
- Include examples in doc comments for complex functions; examples for
  `rblue-api` traits should show the mock backend (`rblue-mock`) so doctests
  run without hardware.

Example doc comment:

````rust
/// Discover peripherals advertising `service_uuid`.
///
/// # Arguments
///
/// * `adapter` - Adapter handle obtained from `Adapter::default()`
/// * `service_uuid` - Only peripherals advertising this service are yielded
///
/// # Returns
///
/// A stream of `(Peripheral, AdvertisementData)` pairs as they are seen
///
/// # Errors
///
/// Returns `Error::AdapterUnavailable` if no Bluetooth adapter is present
/// Returns `Error::ScanDenied` if the platform has not granted Bluetooth scan permission
///
/// # Examples
///
/// ```
/// use rblue::prelude::*;
///
/// # async fn example(adapter: impl Adapter) -> Result<(), rblue::Error> {
/// let mut scan = adapter.scan(&UUID::from_u128(0x180F))?;
/// while let Some((peripheral, adv)) = scan.next().await {
///     println!("found {} rssi {}", adv.local_name(), adv.rssi());
/// }
/// # Ok(())
/// # }
/// ```
pub fn scan(&self, service_uuid: &Uuid) -> Result<ScanStream, Error> {
````

## Type System

- **MUST** leverage Rust's type system to prevent bugs at compile time
- **NEVER** use `.unwrap()` in library code; use `.expect()` only for invariant
  violations with a descriptive message
- **MUST** use meaningful custom error types with `thiserror`
- Use newtypes to distinguish semantically different values of the same
  underlying type (e.g. `Uuid`, `Rssi`, `AttHandle` — a raw `u16` is not a
  handle)
- Prefer `Option<T>` over sentinel values

## Error Handling

- **NEVER** use `.unwrap()` in production code paths
- **MUST** use `Result<T, E>` for fallible operations
- **MUST** use `thiserror` for defining error types in library crates and
  `anyhow` for application-level errors only
- **MUST** propagate errors with `?` operator where appropriate
- Provide meaningful error messages with context using `.context()` from
  `anyhow` (application code only)
- Platform errors MUST be mapped into the unified `rblue-api` error model at
  the backend boundary; raw WinRT/`NSError`/DBus/JNI error types MUST NOT leak
  through public APIs.

## Function Design

- **MUST** keep functions focused on a single responsibility
- **MUST** prefer borrowing (`&T`, `&mut T`) over ownership when possible
- Limit function parameters to 5 or fewer; use a config struct for more
- Return early to reduce nesting
- Use iterators and combinators over explicit loops where clearer

## Struct and Enum Design

- **MUST** keep types focused on a single responsibility
- **MUST** derive common traits: `Debug`, `Clone`, `PartialEq` where appropriate
- Use `#[derive(Default)]` when a sensible default exists
- Prefer composition over inheritance-like patterns
- Use builder pattern for complex struct construction
- Make fields private by default; provide accessor methods when needed

## Async and Concurrency

- **MUST** use `Send` and `Sync` bounds appropriately
- **NEVER** block the async reactor: platform callbacks arrive on foreign
  threads (WinRT threadpool, CoreBluetooth delegate queue, DBus worker, JNI
  threads, ArkTS JS thread). Bridge them into the async world with channels
  and event loops — NOT with `spawn_blocking` polling hacks, busy-waits, or
  `std::thread::sleep`.
- **MUST** use bounded channels in production paths; unbounded channels are a
  memory leak waiting for a chatty peripheral.
- Avoid `Mutex` when `RwLock` or lock-free alternatives are appropriate; on the
  callback bridge, prefer message passing over shared state.
- Do not hold a lock across an `.await` point.
- `rblue-api` traits and types MUST NOT depend on tokio; accept streams/sinks
  generically or define the minimal trait surface needed.

## unsafe and FFI

- **NEVER** use `unsafe` in `rblue-api` or `rblue-mock`.
- Platform backends and FFI facades may use `unsafe` where the platform
  requires it; prefer safe wrapper crates (`objc2`, `windows`, `bluer`) over
  raw FFI.
- **MUST** document safety invariants: every `unsafe` block gets a
  `// SAFETY:` comment explaining why it is sound, and every `unsafe fn` gets
  a `# Safety` doc section.
- FFI callbacks (JNI, Node-API `ThreadsafeFunction`, UniFFI callback
  interfaces) MUST NOT synchronously re-enter or block the host runtime thread;
  hand the event to the async side and return.
- Keep lifetimes of FFI-owned data explicit: data crossing the boundary is
  either copied, leaked-and-documented, or owned by a Drop guard — never
  borrowed from a temporary.

## Testing

- **MUST** write unit tests for all new functions and types
- **MUST** use `rblue-mock` for tests of core logic; **NEVER** require real
  Bluetooth hardware in CI
- Hardware-dependent integration tests MUST be `#[ignore]`-gated or behind an
  explicit feature flag, and documented
- **MUST** use the built-in `#[test]` / `#[tokio::test]` attributes and
  `cargo test`
- Follow the Arrange-Act-Assert pattern
- Do not commit commented-out tests
- Use `#[cfg(test)]` modules for test code
- Every backend MUST pass the same contract test suite defined against
  `rblue-api` semantics (run against the mock by default; per-backend runs are
  opt-in). When you add an API behavior, extend the contract suite.

## Imports and Dependencies

- **MUST** avoid wildcard imports (`use module::*`) except for preludes, test
  modules (`use super::*`), and prelude re-exports
- **MUST** document dependencies in `Cargo.toml` with version constraints
- Use `cargo` for dependency management
- Organize imports: standard library, external crates, local modules
- Use `rustfmt` to automate import formatting

## Rust Best Practices

- **MUST** call `.clone()` explicitly on non-`Copy` types; avoid hidden clones
  in closures and iterators
- **MUST** use pattern matching exhaustively; avoid catch-all `_` patterns when
  possible — especially on GATT opcodes and platform status codes, where a
  catch-all silently swallows new protocol values
- **MUST** use `format!` macro for string formatting
- Use iterators and iterator adapters over manual loops
- Use `enumerate()` instead of manual counter variables
- Prefer `if let` and `while let` for single-pattern matching

## Memory and Performance

- **MUST** avoid unnecessary allocations; prefer `&str` over `String` when
  possible
- **MUST** use `Cow<'_, str>` when ownership is conditionally needed
- Use `Vec::with_capacity()` when the size is known (e.g. GATT payload reads)
- Prefer stack allocation over heap when appropriate
- Use `Arc` and `Rc` judiciously; prefer borrowing
- On `wasm32` and embedded-adjacent paths, treat allocation and copies as
  first-order costs.

## Benchmarking and Optimization

- **NEVER** run benchmarks in parallel, as the benchmarks will compete for
  resources and the results will be invalid
- **NEVER** game the benchmarks. Do not manipulate the benchmarks themselves to
  satisfy any required performance constraints
- **NEVER** run benchmarks with `target-cpu=native` or any other `RUSTFLAGS`
- If benchmarking one backend against another, ensure the benchmarks are
  apples-to-apples comparisons; include the mock backend as the zero-hardware
  baseline
- Ensure benchmark tests are independent. If the tests are dependent due to a
  feature (e.g. caching), ensure the feature is disabled

## Security and Privacy

- **NEVER** store secrets, API keys, or passwords in code. Only store them in
  `.env`
  - Ensure `.env` is declared in `.gitignore`
- **MUST** use environment variables for sensitive configuration via
  `dotenvy` or `std::env`
- **NEVER** log sensitive information (passwords, tokens, PII)
- Bluetooth identifiers are personal data: **NEVER** log MAC addresses,
  stable advertisement payloads, or user-readable device names at info level
  or above in library code; use debug/trace with a documentation note
- Use `secrecy` crate for sensitive data types if key material is ever handled
  (pairing/bonding, Phase 2)

## Version Control

- **MUST** write clear, descriptive commit messages
- **NEVER** commit commented-out code; delete it
- **NEVER** commit debug `println!` statements or `dbg!` macros
- **NEVER** commit credentials or sensitive data
- **NEVER** commit build artifacts (`.so`, `.aar`, `.xcframework`, `pkg/`
  output) — `bindings/` projects build them locally; check `.gitignore` when
  unsure

## Tools

- **MUST** use `rustfmt` for code formatting
- **MUST** use `clippy` for linting and follow its suggestions
- **MUST** ensure code compiles with no warnings (use `-D warnings` flag in
  CI, not `#![deny(warnings)]` in source)
- Use `cargo` for building, testing, and dependency management
- Use `cargo test` for running tests
- Use `cargo doc` for generating documentation
- Cross-target checks before handing off platform-agnostic changes:
  `cargo check --workspace` plus
  `cargo check --workspace --exclude rblue-winrt --exclude rblue-corebluetooth --exclude rblue-bluez --exclude rblue-android --target wasm32-unknown-unknown`
  (the platform shell crates and their btleplug dependency are single-platform
  by design and MUST NOT be pulled into the wasm32 matrix)
- **NEVER** use the `Explore` tool for `Cargo.lock`: it is large and
  irrelevant. Read `Cargo.lock` **ONLY** if it's extremely relevant.

## Binding Rebuilds

Rebuild downstream bindings whenever the Rust code they wrap changes:

- If `rblue-ffi` is touched, regenerate the Kotlin/Swift UniFFI bindings.
- If `rblue-napi` or `rblue-ohos` is touched, rebuild the OHOS `.so`
  (ohos-rs toolchain) and refresh `bindings/arkts`.
- If `rblue-web` or web paths in the aggregate crate are touched, rebuild the
  WASM package (`wasm-pack build`).
- Generated binding code is not hand-edited; change the Rust definitions and
  regenerate.

## Before Committing

- [ ] All tests pass (`cargo test`)
- [ ] No compiler warnings (`cargo build`)
- [ ] Clippy passes (`cargo clippy -- -D warnings`)
- [ ] Code is formatted (`cargo fmt --check`)
- [ ] If `rblue-ffi`/`rblue-napi`/`rblue-web` code was touched, the
      corresponding bindings were regenerated/rebuilt
- [ ] All public items have doc comments
- [ ] No commented-out code or debug statements
- [ ] No hardcoded credentials
- [ ] No new dependency violates the portability rule (compiles on wasm32/OHOS
      or is properly gated)

---

**Remember:** Prioritize clarity and maintainability over cleverness. This is
your core directive.
