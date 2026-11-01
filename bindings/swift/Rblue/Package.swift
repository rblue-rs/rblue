// swift-tools-version: 5.9
//
// Swift package wrapping the rblue UniFFI bindings.
//
// Layout:
//   Rblue.xcframework       static XCFramework built by
//                           examples/ios/scripts/build-frameworks.sh
//                           (gitignored; must exist before resolving).
//   Sources/Rblue           generated UniFFI Swift module (`Rblue.swift`,
//                           script-synced from bindings/swift/generated).
//
// The `RblueFFI` binary target carries the C FFI header plus a
// `module.modulemap` defining the `rblue_ffiFFI` C module that the generated
// Swift code imports. The linker settings attach what the Rust static
// library needs at final-link time (CoreBluetooth for the backend, libobjc
// and libc++ for the Rust runtime).
import PackageDescription

let package = Package(
    name: "Rblue",
    platforms: [
        .iOS(.v16),
    ],
    products: [
        .library(name: "Rblue", targets: ["Rblue"]),
    ],
    targets: [
        .binaryTarget(name: "RblueFFI", path: "Rblue.xcframework"),
        .target(
            name: "Rblue",
            dependencies: ["RblueFFI"],
            path: "Sources/Rblue",
            linkerSettings: [
                .linkedLibrary("c++"),
                .linkedLibrary("objc"),
                .linkedFramework("CoreBluetooth"),
                .linkedFramework("Foundation"),
            ]
        ),
    ]
)
