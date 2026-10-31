//! The concrete `cargo xtask` command implementations.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context};
use indicatif::{ProgressBar, ProgressStyle};

use crate::preflight;
use crate::uniffi::Language;

/// Documentation pointer used in `bind-arkts` errors.
const OHOS_DOCS: &str = "crates/rblue-ohos/docs/platform-notes.md";

/// Device ABI target of `bind-arkts` (HarmonyOS NEXT real devices).
const OHOS_TARGET: &str = "aarch64-unknown-linux-ohos";

/// `cargo` profile flags for the requested mode.
fn profile_args(release: bool) -> &'static [&'static str] {
    if release {
        &["--release"]
    } else {
        &[]
    }
}

/// A steady-tick spinner carrying a context-sensitive message.
fn spinner(message: &str) -> ProgressBar {
    let bar = ProgressBar::new_spinner();
    let style = ProgressStyle::with_template("{spinner} {msg}").expect("static spinner template");
    bar.set_style(style);
    bar.set_message(message.to_owned());
    bar.enable_steady_tick(Duration::from_millis(120));
    bar
}

/// `bind-kotlin` / `bind-swift`: build `rblue-ffi`, then generate the
/// language's bindings from the compiled artifact with the workspace
/// `uniffi-bindgen` bin (library mode; see `crates/rblue-ffi`).
///
/// # Errors
///
/// Missing tooling or a failed build/generation step.
pub fn bind_ffi(language: Language, out_dir: PathBuf, release: bool) -> anyhow::Result<()> {
    let root = preflight::workspace_root()?;
    let profile = if release { "release" } else { "debug" };

    let bar = spinner(&format!("building rblue-ffi ({profile})"));
    preflight::checked(
        preflight::cargo()
            .arg("build")
            .arg("-p")
            .arg("rblue-ffi")
            .args(profile_args(release)),
        "cargo build -p rblue-ffi",
    )?;
    bar.finish_with_message("rblue-ffi built");

    let artifact = preflight::ffi_cdylib(&root, release)?;
    let out_dir = absolute(&root, &out_dir)?;
    let bar = spinner(&format!(
        "generating {} bindings with uniffi-bindgen",
        language.flag()
    ));
    preflight::checked(
        preflight::cargo()
            .args([
                "run",
                "-q",
                "-p",
                "rblue-ffi",
                "--bin",
                "uniffi-bindgen",
                "--",
            ])
            .args(["generate", "--library"])
            .arg(&artifact)
            .args(["--language", language.flag()])
            .arg("--out-dir")
            .arg(&out_dir),
        "uniffi-bindgen generate",
    )?;
    bar.finish_with_message(format!(
        "{} bindings written to {}",
        language.flag(),
        out_dir.display()
    ));
    Ok(())
}

/// `bind-ts`: build the aggregate `rblue` crate (feature `web`) for wasm32
/// with wasm-pack into `<out>/pkg`.
///
/// # Errors
///
/// Missing `wasm-pack` or a failed build.
pub fn bind_ts(out_dir: PathBuf, release: bool) -> anyhow::Result<()> {
    preflight::require_tool(
        "wasm-pack",
        "install it with `cargo install wasm-pack -f` (or `npm install -g wasm-pack`)",
    )?;
    let root = preflight::workspace_root()?;
    let out_dir = absolute(&root, &out_dir)?.join("pkg");
    let mode = if release { "release" } else { "dev" };

    let bar = spinner(&format!(
        "building rblue (web feature) with wasm-pack ({mode}) for wasm32-unknown-unknown"
    ));
    let mut command = Command::new("wasm-pack");
    command
        .arg("build")
        .arg(root.join("crates/rblue"))
        .args(["--target", "web"])
        .args(["--out-dir"])
        .arg(&out_dir)
        .args(["--out-name", "rblue"])
        .args(if release {
            vec!["--release"]
        } else {
            vec!["--dev"]
        })
        .args(["--", "--features", "web"]);
    preflight::checked(
        &mut command,
        "wasm-pack build crates/rblue --target web -- --features web",
    )?;
    bar.finish_with_message(format!("wasm package written to {}", out_dir.display()));
    Ok(())
}

/// `bind-arkts`: build the `rblue-ohos` Node-API facade for arm64-v8a
/// devices. Requires `OHOS_NDK_HOME` (the NDK supplies the OHOS clang linker)
/// and the `aarch64-unknown-linux-ohos` rustup target; see
/// `crates/rblue-ohos/docs/platform-notes.md`.
///
/// # Errors
///
/// Missing NDK/target, or a failed build. The HAR packaging itself (hvigor)
/// is not part of this command.
pub fn bind_arkts(out_dir: PathBuf, release: bool) -> anyhow::Result<()> {
    let ndk = preflight::require_env(
        "OHOS_NDK_HOME",
        "the OHOS NDK supplies the linker for the Node-API cdylib",
        OHOS_DOCS,
    )?;
    preflight::require_tool("rustup", "install it from https://rustup.rs")?;
    if !preflight::rustup_target_installed(OHOS_TARGET)? {
        bail!("{OHOS_TARGET} is not installed; run `rustup target add {OHOS_TARGET}`");
    }
    let linker = Path::new(&ndk)
        .join("llvm")
        .join("bin")
        .join("aarch64-unknown-linux-ohos-clang");
    if !linker.is_file() {
        bail!(
            "the OHOS NDK linker was not found at {} (expected the llvm/ layout of the HarmonyOS NDK)",
            linker.display()
        );
    }

    let root = preflight::workspace_root()?;
    let profile = if release { "release" } else { "debug" };
    let bar = spinner(&format!(
        "building rblue-ohos napi facade ({profile}, {OHOS_TARGET})"
    ));
    let mut command = preflight::cargo();
    command
        .arg("build")
        .arg("-p")
        .arg("rblue-ohos")
        .arg("--features")
        .arg("napi")
        .arg("--target")
        .arg(OHOS_TARGET)
        .args(profile_args(release))
        .env("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER", &linker)
        .env("CC_aarch64_unknown_linux_ohos", &linker)
        .env(
            "AR_aarch64_unknown_linux_ohos",
            Path::new(&ndk).join("llvm").join("bin").join("llvm-ar"),
        );
    preflight::checked(
        &mut command,
        "cargo build -p rblue-ohos --features napi --target aarch64-unknown-linux-ohos",
    )?;
    bar.finish_with_message("rblue-ohos built");

    // The HAR ships the cdylib under libs/arm64-v8a/. rblue-ohos does not
    // declare a cdylib crate-type yet (that lands with the M4 HAR packaging),
    // so the artifact may legitimately be absent after a successful build.
    let so = root
        .join("target")
        .join(OHOS_TARGET)
        .join(profile)
        .join("librblue_ohos.so");
    if so.is_file() {
        let out_dir = absolute(&root, &out_dir)?.join("libs").join("arm64-v8a");
        std::fs::create_dir_all(&out_dir)
            .context("failed to create the bindings/arkts libs directory")?;
        let destination = out_dir.join("librblue_ohos.so");
        std::fs::copy(&so, &destination).context("failed to copy the built cdylib")?;
        tracing::info!("cdylib copied to {}", destination.display());
    } else {
        tracing::info!(
            "no librblue_ohos.so was produced; rblue-ohos does not declare a cdylib \
             crate-type yet (M4 HAR packaging); the build artifacts are under target/{OHOS_TARGET}"
        );
    }
    Ok(())
}

/// `dist`: preflight the tooling of every step (all problems reported at
/// once), then run the four binding steps in sequence.
///
/// # Errors
///
/// The first preflight gap or the first failing step.
pub fn dist(release: bool) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    if let Err(error) = preflight::require_tool(
        "wasm-pack",
        "install it with `cargo install wasm-pack -f` (or `npm install -g wasm-pack`)",
    ) {
        problems.push(error.to_string());
    }
    if let Err(error) = preflight::require_env(
        "OHOS_NDK_HOME",
        "the OHOS NDK supplies the linker for the Node-API cdylib",
        OHOS_DOCS,
    ) {
        problems.push(error.to_string());
    }
    match preflight::require_tool("rustup", "install it from https://rustup.rs")
        .and_then(|_| preflight::rustup_target_installed(OHOS_TARGET))
    {
        Ok(true) => {}
        Ok(false) => problems.push(format!(
            "{OHOS_TARGET} is not installed; run `rustup target add {OHOS_TARGET}`"
        )),
        Err(error) => problems.push(error.to_string()),
    }
    if !problems.is_empty() {
        bail!(
            "dist preflight found {} problem(s):\n - {}",
            problems.len(),
            problems.join("\n - ")
        );
    }

    bind_ffi(
        Language::Kotlin,
        PathBuf::from(Language::Kotlin.default_out_dir()),
        release,
    )?;
    bind_ffi(
        Language::Swift,
        PathBuf::from(Language::Swift.default_out_dir()),
        release,
    )?;
    bind_ts(PathBuf::from("bindings/ts"), release)?;
    bind_arkts(PathBuf::from("bindings/arkts"), release)?;
    tracing::info!("dist complete");
    Ok(())
}

/// Resolves `dir` against the workspace root when relative.
fn absolute(root: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    if dir.is_absolute() {
        Ok(dir.to_owned())
    } else {
        Ok(root.join(dir))
    }
}
