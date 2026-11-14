// swift-tools-version: 5.9
// Flutter 3.41.7 discovers this manifest at ios/rblue_flutter/Package.swift.

import PackageDescription

let rblueAbiRoots = [
    "_rblue_new",
    "_rblue_free",
    "_rblue_request",
    "_rblue_shutdown",
    "_rblue_set_event_callback",
    "_rblue_set_owned_response_callback",
    "_rblue_set_response_callback",
    "_rblue_free_string",
    "_rblue_free_payload",
]

let rblueAbiLinkerFlags = rblueAbiRoots.flatMap { symbol in
    ["-Xlinker", "-u", "-Xlinker", symbol]
}

let package = Package(
    name: "rblue_flutter",
    platforms: [
        .iOS("15.0"),
    ],
    products: [
        // Flutter derives this hyphenated product name from the pub package name.
        .library(name: "rblue-flutter", targets: ["rblue_flutter"]),
    ],
    dependencies: [
        .package(name: "FlutterFramework", path: "../FlutterFramework"),
    ],
    targets: [
        // Local binary target containing arm64 iOS device and simulator slices.
        .binaryTarget(name: "RblueC", path: "RblueC.xcframework"),
        .target(
            name: "RblueAnchor",
            dependencies: ["RblueC"],
            path: "Sources/RblueAnchor",
            publicHeadersPath: "include"
        ),
        .target(
            name: "rblue_flutter",
            dependencies: [
                "RblueAnchor",
                .product(name: "FlutterFramework", package: "FlutterFramework"),
            ],
            path: "Sources/rblue_flutter",
            resources: [
                .process("PrivacyInfo.xcprivacy"),
            ],
            linkerSettings: [
                .linkedFramework("CoreBluetooth"),
                .linkedFramework("Foundation"),
                .linkedLibrary("c++"),
                .linkedLibrary("objc"),
                // Dart resolves these from DynamicLibrary.process(). The anchor
                // references every ABI entry point, while -u keeps each symbol
                // rooted through the consuming application's final dead-strip.
                .unsafeFlags(rblueAbiLinkerFlags),
            ]
        ),
    ]
)
