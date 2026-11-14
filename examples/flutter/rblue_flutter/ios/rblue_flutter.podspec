# rblue_flutter iOS packaging: the published package contains arm64 device and
# Apple Silicon simulator static slices. The C ABI roots are retained explicitly
# so Dart FFI resolves them through DynamicLibrary.process(), while the linker
# pulls the Rust archive members on demand.
#
# Reproducible source, build commands, checksums, and license inventory live in
# native/BUILDING.md and THIRD_PARTY_NOTICES.md.
Pod::Spec.new do |s|
  s.name             = 'rblue_flutter'
  s.version          = '0.0.1-dev.2'
  s.summary          = 'Experimental Flutter BLE plugin backed by the rblue Rust C ABI.'
  s.description      = <<-DESC
Early experimental Flutter binding for rblue. Packages prebuilt rblue-c native
libraries for Android and arm64 iOS device/Apple Silicon simulator targets.
                       DESC
  s.homepage         = 'https://github.com/rblue-rs/rblue'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'rblue contributors' => 'https://github.com/rblue-rs/rblue' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.public_header_files = 'Classes/include/rblue.h'
  s.private_header_files = 'Classes/include/rblue_flutter_anchor.h'
  s.header_mappings_dir = 'Classes/include'
  # CocoaPods selects the matching static XCFramework slice and propagates it
  # correctly for both static-library and framework integrations.
  s.vendored_frameworks = 'rblue_flutter/RblueC.xcframework'
  s.resource_bundles = {
    'rblue_flutter_privacy' => ['Resources/PrivacyInfo.xcprivacy'],
  }
  s.dependency 'Flutter'
  # CocoaPods turns these into -framework/-l flags on every consuming target,
  # including Runner where Dart searches the final image at runtime.
  s.frameworks      = 'CoreBluetooth', 'Foundation'
  s.libraries       = 'c++', 'objc'
  # 15.0: the minimum deployment target current Xcode toolchains accept
  # (Flutter 3.41 era); the Rust static slice itself only needs 13+.
  s.platform         = :ios, '15.0'
  s.swift_version    = '5.0'

  # Dart FFI discovers the C ABI dynamically, so no Swift/ObjC object refers to
  # these symbols directly. `-u` roots the public ABI without force-loading the
  # entire Rust archive; force_load would include mutually redundant Rust
  # codegen objects and produces duplicate symbols with current toolchains.
  abi_roots = %w[
    rblue_new
    rblue_free
    rblue_request
    rblue_shutdown
    rblue_set_event_callback
    rblue_set_owned_response_callback
    rblue_set_response_callback
    rblue_free_string
    rblue_free_payload
  ].map { |symbol| "-Wl,-u,_#{symbol}" }.join(' ')

  # SDK-qualified keys replace (not merge with) the unqualified value, so the
  # framework and ABI-root flags are repeated in each variant. They live in both
  # pod and user target xcconfig: the pod target links the native archive into
  # its framework/static product, and the app's final link keeps the names rooted
  # for DynamicLibrary.process() even when dead stripping is enabled.
  link_flags = "-framework CoreBluetooth -framework Foundation -lc++ -lobjc #{abi_roots}"
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386 x86_64',
    'OTHER_LDFLAGS[sdk=iphoneos*]' => "$(inherited) #{link_flags}",
    'OTHER_LDFLAGS[sdk=iphonesimulator*]' => "$(inherited) #{link_flags}",
  }
  s.user_target_xcconfig = {
    'OTHER_LDFLAGS' => "$(inherited) #{link_flags}",
  }

  s.prepare_command = <<-CMD
    test -f rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a || { \
      echo "rblue_flutter: published package is missing the iOS device XCFramework slice; reinstall it or report a packaging bug"; exit 1; }
    test -f rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a || { \
      echo "rblue_flutter: published package is missing the iOS simulator XCFramework slice; reinstall it or report a packaging bug"; exit 1; }
    test -f Resources/PrivacyInfo.xcprivacy || { \
      echo "rblue_flutter: published package is missing the iOS privacy manifest; reinstall it or report a packaging bug"; exit 1; }
  CMD
end
