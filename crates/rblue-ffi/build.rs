// The JNI `nativeInit` entry lives in the rblue-android rlib. Android lld can
// keep that object via `#[used]` but still omit it from this cdylib's dynamic
// symbol table, so JNI lookup fails. Force the symbol to be pulled in and
// exported from the final shared library.
fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if std::env::var("CARGO_FEATURE_ANDROID").is_ok() && target.contains("android") {
        let symbol = "Java_com_rblue_RblueNative_nativeInit";
        println!("cargo:rustc-cdylib-link-arg=-Wl,-u,{symbol}");
        println!("cargo:rustc-cdylib-link-arg=-Wl,--export-dynamic-symbol={symbol}");
    }
}
