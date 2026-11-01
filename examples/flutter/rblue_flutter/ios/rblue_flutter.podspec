# rblue_flutter iOS packaging: the device and simulator static slices of the
# rblue C ABI are force_load-ed into the Runner binary, so the Dart FFI finds
# the symbols through DynamicLibrary.process() at runtime.
#
# The slices under Libs/ are build artifacts produced by
# examples/flutter/tools/build-libs.sh; prepare_command fails with instructions
# when they are missing.
Pod::Spec.new do |s|
  s.name             = 'rblue_flutter'
  s.version          = '0.1.0'
  s.summary          = 'rblue BLE example plugin: prebuilt rblue-c static library plus Flutter glue.'
  s.description      = <<-DESC
Local plugin of the rblue Flutter example. Packages librblue_c.a (device and
simulator slices) and exposes the rblue C ABI to Dart via dart:ffi.
                       DESC
  s.homepage         = 'https://github.com/rblue-rs/rblue'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'rblue contributors' => 'rblue@example.com' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'Flutter'
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
    'OTHER_LDFLAGS[sdk=iphoneos*]' => '$(inherited) -framework CoreBluetooth'
      ' -framework Foundation -lobjc -force_load $(PODS_TARGET_SRCROOT)/Libs/ios/librblue_c.a',
    'OTHER_LDFLAGS[sdk=iphonesimulator*]' => '$(inherited) -framework CoreBluetooth'
      ' -framework Foundation -lobjc -force_load $(PODS_TARGET_SRCROOT)/Libs/ios-sim/librblue_c.a',
  }

  s.prepare_command = <<-CMD
    test -f Libs/ios/librblue_c.a || { \\
      echo "rblue_flutter: missing Libs/ios/librblue_c.a - run" \\
           "examples/flutter/tools/build-libs.sh first"; exit 1; }
    test -f Libs/ios-sim/librblue_c.a || { \\
      echo "rblue_flutter: missing Libs/ios-sim/librblue_c.a - run" \\
           "examples/flutter/tools/build-libs.sh first"; exit 1; }
  CMD
end
