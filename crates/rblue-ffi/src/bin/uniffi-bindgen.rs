//! Standalone UniFFI binding generator for this crate.
//!
//! The Kotlin and Swift bindings are generated in library mode from the
//! compiled `rblue-ffi` artifact (proc-macro definitions, no UDL):
//!
//! ```text
//! cargo build -p rblue-ffi --release
//! cargo run -p rblue-ffi --bin uniffi-bindgen -- \
//!     generate --library target/release/librblue_ffi.dylib \
//!     --language kotlin --out-dir bindings/kotlin
//! ```
//!
//! `cargo xtask bind-kotlin` / `bind-swift` wrap exactly this flow. The tool
//! is a host binary; on wasm32 (where the facade itself is compiled out) it
//! exits with an error.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    uniffi::uniffi_bindgen_main();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("uniffi-bindgen is a host tool; run it on a desktop target");
    std::process::exit(2);
}
