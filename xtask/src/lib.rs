//! Build and packaging orchestration for the rblue workspace.
//!
//! Everything that would otherwise be a shell script lives here behind a
//! `cargo xtask <cmd>` entry point. The command catalog:
//!
//! | Command | Tooling checked up front | What it does |
//! |---|---|---|
//! | `bind-kotlin` | cargo (inherent) | builds `rblue-ffi`, then runs the
//!   workspace `uniffi-bindgen` bin in library mode to emit Kotlin bindings
//!   into `bindings/kotlin` |
//! | `bind-swift` | cargo (inherent) | same, emitting Swift bindings into
//!   `bindings/swift` |
//! | `bind-ts` | `wasm-pack` | builds the aggregate `rblue` crate with the
//!   `web` feature for `wasm32-unknown-unknown` into `bindings/ts/pkg` |
//! | `bind-arkts` | `OHOS_NDK_HOME`, `rustup` + the
//!   `aarch64-unknown-linux-ohos` target | builds the `rblue-ohos` Node-API
//!   facade for arm64-v8a devices |
//! | `dist` | all of the above (checked before anything runs) | the four
//!   steps in sequence |
//!
//! Every command fails fast with an install hint when its tooling is
//! missing; long operations get `indicatif` progress bars with
//! context-sensitive messages.

mod preflight;
mod steps;

use std::path::PathBuf;

use clap::Subcommand;

/// `bind-*` arguments shared by every binding command.
#[derive(clap::Args, Debug)]
pub struct BindArgs {
    /// Output directory override. Defaults to `bindings/<target>` below the
    /// workspace root.
    #[arg(long)]
    pub out_dir: Option<PathBuf>,

    /// Build in release mode where a native build is involved.
    #[arg(long)]
    pub release: bool,
}

/// `dist` arguments.
#[derive(clap::Args, Debug)]
pub struct DistArgs {
    /// Build in release mode.
    #[arg(long)]
    pub release: bool,
}

/// The `cargo xtask` subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Generate Kotlin bindings (UniFFI, library mode) for rblue-ffi.
    BindKotlin(BindArgs),
    /// Generate Swift bindings (UniFFI, library mode) for rblue-ffi.
    BindSwift(BindArgs),
    /// Build the WASM package for the rblue `web` feature with wasm-pack.
    BindTs(BindArgs),
    /// Build the OHOS Node-API facade of rblue-ohos (needs OHOS_NDK_HOME).
    BindArkts(BindArgs),
    /// Run all four binding steps in sequence (preflighting all tooling
    /// first).
    Dist(DistArgs),
}

/// Runs one command.
///
/// # Errors
///
/// Any tooling or build failure, with context about the failing step.
pub fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::BindKotlin(args) => steps::bind_ffi(
            uniffi::Language::Kotlin,
            resolve(args.out_dir, uniffi::Language::Kotlin.default_out_dir()),
            args.release,
        ),
        Command::BindSwift(args) => steps::bind_ffi(
            uniffi::Language::Swift,
            resolve(args.out_dir, uniffi::Language::Swift.default_out_dir()),
            args.release,
        ),
        Command::BindTs(args) => steps::bind_ts(resolve(args.out_dir, "bindings/ts"), args.release),
        Command::BindArkts(args) => {
            steps::bind_arkts(resolve(args.out_dir, "bindings/arkts"), args.release)
        }
        Command::Dist(args) => steps::dist(args.release),
    }
}

/// `--out-dir` override or the documented default below the workspace root.
fn resolve(dir: Option<PathBuf>, default: &str) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from(default))
}

/// The uniffi-bindgen target languages this xtask drives.
///
/// Named here so `steps` does not need a uniffi dependency of its own.
mod uniffi {
    /// Languages the workspace `uniffi-bindgen` bin can emit.
    #[derive(Clone, Copy, Debug)]
    pub enum Language {
        /// Kotlin bindings (Android/JVM).
        Kotlin,
        /// Swift bindings (iOS/macOS).
        Swift,
    }

    impl Language {
        /// `--language` value of `uniffi-bindgen generate`.
        pub fn flag(self) -> &'static str {
            match self {
                Self::Kotlin => "kotlin",
                Self::Swift => "swift",
            }
        }

        /// Default output directory below the workspace root.
        pub fn default_out_dir(self) -> &'static str {
            match self {
                Self::Kotlin => "bindings/kotlin",
                Self::Swift => "bindings/swift",
            }
        }
    }
}
