//! Up-front tooling checks and process helpers for the xtask steps.
//!
//! Every command verifies its tooling before starting any build so a missing
//! `wasm-pack` or OHOS NDK is reported in seconds instead of after minutes of
//! compiling.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context};

/// The cargo binary driving this xtask (set by cargo itself, falls back to
/// `cargo` on `PATH`).
pub fn cargo() -> Command {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    Command::new(program)
}

/// Absolute workspace root (the parent of the xtask crate directory).
///
/// # Errors
///
/// When the `CARGO_MANIFEST_DIR` of this binary cannot be determined.
pub fn workspace_root() -> anyhow::Result<PathBuf> {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| anyhow!("CARGO_MANIFEST_DIR is not set; run xtask via `cargo xtask`"))?;
    let root = Path::new(&manifest_dir)
        .parent()
        .ok_or_else(|| anyhow!("cannot derive the workspace root from {manifest_dir:?}"))?
        .to_owned();
    Ok(root)
}

/// Resolves `tool` on `PATH`.
///
/// # Errors
///
/// A message naming the tool and its install hint when it is absent.
pub fn require_tool(tool: &str, hint: &str) -> anyhow::Result<PathBuf> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let path = std::env::split_paths(std::env::var_os("PATH").as_deref().unwrap_or_default())
        .map(|dir| dir.join(format!("{tool}{suffix}")))
        .find(|candidate| candidate.is_file());
    path.ok_or_else(|| anyhow!("{tool} was not found on PATH; {hint}"))
}

/// Reads a required environment variable.
///
/// # Errors
///
/// A message naming the variable, its role, and the documentation pointer
/// when unset.
pub fn require_env(name: &str, role: &str, docs: &str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| anyhow!("{name} is not set; {role} (see {docs})"))
}

/// Runs a command, attaching its stderr to the error context on failure.
///
/// # Errors
///
/// When the command cannot spawn or exits non-zero.
pub fn checked(command: &mut Command, what: &str) -> anyhow::Result<()> {
    let output = command
        .output()
        .with_context(|| format!("failed to spawn {what}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim_end();
    bail!("{what} failed ({}):\n{stderr}", output.status);
}

/// `target/{profile}` below the workspace root.
pub fn target_dir(root: &Path, release: bool) -> PathBuf {
    root.join("target")
        .join(if release { "release" } else { "debug" })
}

/// The `librblue_ffi` cdylib artifact for the host OS.
///
/// # Errors
///
/// When the artifact is absent (it is built first by the bind-kotlin/bind-swift
/// steps).
pub fn ffi_cdylib(root: &Path, release: bool) -> anyhow::Result<PathBuf> {
    let file = if cfg!(target_os = "macos") {
        "librblue_ffi.dylib"
    } else if cfg!(target_os = "windows") {
        "rblue_ffi.dll"
    } else {
        "librblue_ffi.so"
    };
    let artifact = target_dir(root, release).join(file);
    if artifact.is_file() {
        Ok(artifact)
    } else {
        Err(anyhow!(
            "{} is missing; the rblue-ffi build did not produce a cdylib",
            artifact.display()
        ))
    }
}

/// `true` when `target` is installed for the active toolchain (via `rustup`).
pub fn rustup_target_installed(target: &str) -> anyhow::Result<bool> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("failed to run rustup")?;
    let installed = String::from_utf8_lossy(&output.stdout);
    Ok(installed.lines().any(|line| line.trim() == target))
}
