# rblue_flutter iOS packaging: the published package contains arm64 device and
# Apple Silicon simulator static slices. They are force_load-ed into the Runner
# binary so Dart FFI resolves the C ABI through DynamicLibrary.process().
#
# Reproducible source, build commands, checksums, and license inventory live in
# native/BUILDING.md and THIRD_PARTY_NOTICES.md.
Pod::Spec.new do |s|
  s.name             = 'rblue_flutter'
  s.version          = '0.0.1-dev.1'
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
  s.dependency 'Flutter'
  # CocoaPods turns these into -framework/-l flags on every consuming target
  # (including Runner, where the final link of the force_load-ed static slice
  # happens) - raw OTHER_LDFLAGS on the pod target alone do not reach it.
  s.frameworks      = 'CoreBluetooth', 'Foundation'
  s.libraries       = 'c++', 'objc'
  # 15.0: the minimum deployment target current Xcode toolchains accept
  # (Flutter 3.41 era); the Rust static slice itself only needs 13+.
  s.platform         = :ios, '15.0'
  s.swift_version    = '5.0'

  # Flutter.framework does not contain an i386 slice.
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    # dart:ffi resolves symbols from the executable's load list, but the
    # linker only pulls object files from a static library when something
    # references them - and nothing does at link time. force_load pulls every
    # object in, per SDK. The Rust static slice references CoreBluetooth
    # (backend) and the ObjC runtime; those must resolve at this link step.
    # SDK-qualified keys replace (not merge with) the unqualified value, so
    # the framework flags are repeated in each variant.
    'OTHER_LDFLAGS[sdk=iphoneos*]' =>
      '$(inherited) -framework CoreBluetooth -framework Foundation -lobjc -force_load $(PODS_TARGET_SRCROOT)/Libs/ios/librblue_c.a',
    'OTHER_LDFLAGS[sdk=iphonesimulator*]' =>
      '$(inherited) -framework CoreBluetooth -framework Foundation -lobjc -force_load $(PODS_TARGET_SRCROOT)/Libs/ios-sim/librblue_c.a',
  }

  s.prepare_command = <<-CMD
    test -f Libs/ios/librblue_c.a || { \\
      echo "rblue_flutter: published package is missing the iOS device library; reinstall it or report a packaging bug"; exit 1; }
    test -f Libs/ios-sim/librblue_c.a || { \\
      echo "rblue_flutter: published package is missing the iOS simulator library; reinstall it or report a packaging bug"; exit 1; }
  CMD
end
