# rblue

**r**ust + **blue**(tooth) — one async Bluetooth LE API across
Windows, macOS, Linux, iOS, Android, Web (Web Bluetooth) and HarmonyOS.

Pronounced **"R-blue"**. Member of the Blue* family: BlueZ, bluer, bluest … rblue.

## Status

🚧 Design phase. The crate name `rblue` is reserved for crates.io; the GitHub
organization will be `rblue-rs` (the exact-name `rblue` namespace is held by an
inactive 2014 account, which GitHub does not release).

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
