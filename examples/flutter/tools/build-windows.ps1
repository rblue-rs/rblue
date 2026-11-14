param(
  [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../../..")).Path
$PackageRoot = Join-Path $RepoRoot "examples/flutter/rblue_flutter"
$SnapshotRoot = Join-Path $PackageRoot "native/source"
$Target = "x86_64-pc-windows-msvc"
$ProfileDirectory = if ($Configuration -eq "release") { "release" } else { "debug" }

$cargoArguments = @(
  "build",
  "--locked",
  "--manifest-path", (Join-Path $SnapshotRoot "Cargo.toml"),
  "--target", $Target,
  "--no-default-features",
  "--features", "winrt",
  "-p", "rblue-c"
)
if ($Configuration -eq "release") {
  $cargoArguments += "--release"
} elseif ($Configuration -ne "debug") {
  throw "Configuration must be 'debug' or 'release'."
}

& cargo @cargoArguments
if ($LASTEXITCODE -ne 0) {
  throw "cargo failed with exit code $LASTEXITCODE"
}

$builtDll = Join-Path $SnapshotRoot "target/$Target/$ProfileDirectory/rblue_c.dll"
$packagedDirectory = Join-Path $PackageRoot "windows/x64"
$packagedDll = Join-Path $packagedDirectory "rblue_c.dll"
if (-not (Test-Path $builtDll -PathType Leaf)) {
  throw "Expected Rust DLL was not produced: $builtDll"
}
New-Item -ItemType Directory -Force -Path $packagedDirectory | Out-Null
Copy-Item -Force $builtDll $packagedDll

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio/Installer/vswhere.exe"
if (-not (Test-Path $vswhere -PathType Leaf)) {
  throw "vswhere.exe was not found; install Visual Studio with the Desktop development with C++ workload."
}
$visualStudioRoot = & $vswhere -latest -products * `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath
if (-not $visualStudioRoot) {
  throw "A Visual Studio installation with x64 MSVC tools was not found."
}
$dumpbin = Get-ChildItem `
  (Join-Path $visualStudioRoot "VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe") |
  Sort-Object FullName |
  Select-Object -Last 1
if (-not $dumpbin) {
  throw "dumpbin.exe was not found under $visualStudioRoot."
}

$exports = (& $dumpbin.FullName /nologo /exports $packagedDll) -join "`n"
$requiredExports = @(
  "rblue_new",
  "rblue_free",
  "rblue_request",
  "rblue_shutdown",
  "rblue_set_event_callback",
  "rblue_set_owned_response_callback",
  "rblue_set_response_callback",
  "rblue_free_string",
  "rblue_free_payload"
)
foreach ($symbol in $requiredExports) {
  if ($exports -notmatch "(?m)\b$([Regex]::Escape($symbol))\b") {
    throw "Packaged DLL is missing export: $symbol"
  }
}

$hash = (Get-FileHash -Algorithm SHA256 $packagedDll).Hash.ToLowerInvariant()
Write-Host "Packaged and verified $packagedDll"
Write-Host "SHA-256: $hash"
Write-Host "Verified $($requiredExports.Count) rblue C ABI exports."
