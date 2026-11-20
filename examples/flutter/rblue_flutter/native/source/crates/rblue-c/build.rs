//! Keep the JNI `nativeInit` symbol in the Android cdylib dynamic table.
//!
//! rustc feeds the linker a version script for `cdylib` crates. On some
//! Android ABIs (observed on `aarch64-linux-android` with rustc 1.99 and
//! NDK lld), an otherwise-unreferenced `#[no_mangle]` defined in this
//! crate can still be omitted from `.dynsym`. `-u` plus
//! `--export-dynamic-symbol` force that one symbol into the export table.
//! These flags are emitted only when the `android` feature is on and the
//! target is Android, so mock-backend cdylibs (no such symbol) still link.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let android_os = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android");
    let android_feature = std::env::var("CARGO_FEATURE_ANDROID").is_ok();
    if android_os && android_feature {
        const SYMBOL: &str = "Java_com_rblue_RblueNative_nativeInit";
        println!("cargo:rustc-cdylib-link-arg=-Wl,-u,{SYMBOL}");
        println!("cargo:rustc-cdylib-link-arg=-Wl,--export-dynamic-symbol={SYMBOL}");
    }
}
