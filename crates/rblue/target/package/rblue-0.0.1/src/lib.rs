//! # rblue
//!
//! **r**ust + **blue**(tooth) — cross-platform async Bluetooth Low Energy,
//! one unified API across Windows, macOS, Linux, iOS, Android,
//! Web Bluetooth, and HarmonyOS.
//!
//! Pronounced **"R-blue"**; member of the Blue\* family (BlueZ, bluer, bluest).
//!
//! ## This release
//!
//! This is a **name-reservation release**: the `rblue` crate name is now
//! permanently taken on crates.io. The unified API is in design and nothing
//! in `0.0.x` is usable yet — pin nothing, watch the repository instead.
//!
//! ## Planned layering
//!
//! - `rblue-api` — core traits: `Adapter` / `Peripheral` / GATT, error model,
//!   capability negotiation (runtime-agnostic, no_std-friendly later)
//! - platform backends — Windows (WinRT) · macOS/iOS (CoreBluetooth) ·
//!   Linux (BlueZ) · Android (JNI) · Web (web-sys / Web Bluetooth) ·
//!   HarmonyOS (ArkTS driver bridged over Node-API)
//! - bindings — Kotlin (UniFFI) · Swift (UniFFI) · TypeScript (wasm-bindgen) ·
//!   ArkTS (ohos-rs)
