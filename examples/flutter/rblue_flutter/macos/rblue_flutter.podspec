# rblue_flutter macOS packaging: CocoaPods consumes the universal arm64/x86_64
# static archive under Libs. Swift Package Manager consumes the equivalent local
# XCFramework at macos/rblue_flutter/RblueC.xcframework.
Pod::Spec.new do |s|
  s.name             = 'rblue_flutter'
  s.version          = '0.0.1-dev.2'
  s.summary          = 'Experimental Flutter BLE plugin backed by the rblue Rust C ABI.'
  s.description      = <<-DESC
Early experimental Flutter binding for rblue with a CoreBluetooth-backed,
universal macOS native library.
                       DESC
  s.homepage         = 'https://github.com/rblue-rs/rblue'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'rblue contributors' => 'https://github.com/rblue-rs/rblue' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.public_header_files = 'Classes/include/rblue.h'
  s.private_header_files = 'Classes/include/rblue_flutter_anchor.h'
  s.header_mappings_dir = 'Classes/include'
  # CocoaPods selects the universal static XCFramework library and propagates it
  # correctly for both static-library and framework integrations.
  s.vendored_frameworks = 'rblue_flutter/RblueC.xcframework'
  s.resource_bundles = {
    'rblue_flutter_privacy' => ['Resources/PrivacyInfo.xcprivacy'],
  }
  s.dependency 'FlutterMacOS'
  s.frameworks       = 'CoreBluetooth', 'Foundation'
  s.libraries        = 'c++', 'objc'
  s.platform         = :osx, '12.0'
  s.swift_version    = '5.0'

  # Dart FFI discovers these through DynamicLibrary.process(), so root every
  # public ABI symbol explicitly through the consuming application's final link.
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
  link_flags = "-framework CoreBluetooth -framework Foundation -lc++ -lobjc #{abi_roots}"

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'OTHER_LDFLAGS' => "$(inherited) #{link_flags}",
  }
  s.user_target_xcconfig = {
    'OTHER_LDFLAGS' => "$(inherited) #{link_flags}",
  }

  s.prepare_command = <<-CMD
    test -f rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a || { \
      echo "rblue_flutter: published package is missing the universal macOS XCFramework slice; reinstall it or report a packaging bug"; exit 1; }
    test -f Resources/PrivacyInfo.xcprivacy || { \
      echo "rblue_flutter: published package is missing the macOS privacy manifest; reinstall it or report a packaging bug"; exit 1; }
  CMD
end
