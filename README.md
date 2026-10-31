# rblue

**r**ust + **blue**(tooth) — one async Bluetooth LE API across
Windows, macOS, Linux, iOS, Android, Web (Web Bluetooth) and HarmonyOS.

Pronounced **"R-blue"**. Member of the Blue* family: BlueZ, bluer, bluest … rblue.

## Status

🚧 **Implemented through milestone M1** (of the implementation plan): the
unified API, the core value-add layer, the mock backend, and all six platform
backends build and pass the contract test suite. Published on crates.io as
[`rblue`](https://crates.io/crates/rblue) (v0.0.1, name-reservation release).
Source lives at [`rblue-rs/rblue`](https://github.com/rblue-rs/rblue)
(the exact-name `rblue` namespace is held by an inactive 2014 account, which
GitHub does not release).

Not yet landed: native binding packaging (AAR/XCFramework/npm/HAR — M2/M3/M4),
real-hardware test runs, and GATT server / advertising (Phase 2).

## Workspace

| Crate | What lives here |
|---|---|
| `rblue-api` | Frozen trait surface: `Adapter` / `Peripheral` / GATT, error model, capability negotiation, contract test suite |
| `rblue-api` value-add | Connection supervisor (backoff, GATT-133 breaker, `reconnect_by_filter`), per-connection op queue, opt-in service cache, scan debounce, session restore |
| `rblue-mock` | In-memory backend: scripted advertisements, controllable disconnects, error injection |
| `rblue` | Aggregate crate: feature per backend (`mock` default, `winrt`, `corebluetooth`, `bluez`, `android`, `web`, `ohos`), target-OS auto-selection |
| `rblue-winrt` / `rblue-corebluetooth` / `rblue-bluez` / `rblue-android` | Thin shells over the [btleplug fork](https://github.com/rblue-rs/btleplug) (rev-pinned) |
| `rblue-web` | Native Web Bluetooth backend (`web-sys`, wasm32) |
| `rblue-ohos` | HarmonyOS driver track: `BleDriver` transport, napi facade behind a feature |
| `rblue-driver` | The driver-track protocol (req_id, version handshake, binary frames) |
| `rblue-ffi` | UniFFI facade for Kotlin/Swift (async methods, event sink) |
| `xtask` | `cargo xtask bind-kotlin\|bind-swift\|bind-ts\|bind-arkts\|dist` |

Platform facts per backend (capability matrix, error mappings, limitations)
live in each crate's `docs/error-mapping.md` and `docs/platform-notes.md`.

## Try it

```bash
cargo test --workspace                  # 300+ tests, no hardware needed
cargo run -p rblue --example mock_demo  # scan→connect→GATT round-trip on the mock
cargo clippy --workspace --all-targets -- -D warnings
```

## Scope

| Layer | What lives here |
|---|---|
| Core (`rblue-api`) | Unified async traits: `Adapter` / `Peripheral` / GATT, error model, capability negotiation |
| Backends | Windows (WinRT) · macOS/iOS (CoreBluetooth) · Linux (BlueZ) · Android (JNI) · Web (web-sys) · HarmonyOS (ArkTS driver via Node-API) |
| Bindings | Kotlin (UniFFI) · Swift (UniFFI) · TypeScript (wasm-bindgen) · ArkTS (ohos-rs) |

Rust-first core; every platform speaks the same traits. Central (GATT client)
first; GATT server and advertising planned.

## License

TBD (MIT / Apache-2.0 dual license is the working default, consistent with the
Rust ecosystem).
