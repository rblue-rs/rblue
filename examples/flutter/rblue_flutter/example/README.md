# rblue_flutter example

Experimental Android, iOS, macOS, and Windows app for `rblue_flutter`. It
demonstrates permission handling, scanning, connecting, GATT discovery,
read/write, notifications, and a Dart-managed reconnect loop.

Use physical hardware for BLE acceptance. Simulators and hosted CI validate
packaging, linking, and UI behavior only. The package currently includes:

- Android API 24+ on arm64-v8a, armeabi-v7a, and x86_64;
- arm64 iOS 15+ devices and Apple Silicon simulators;
- macOS 12+ on arm64 and x86_64;
- Windows 10 1809+ on x64 MSVC.

See the parent README for platform setup and known limitations.

```bash
flutter pub get
flutter run
```

The included Apple runners contain example Bluetooth usage strings, and the
macOS runner includes the sandbox Bluetooth entitlement. Applications must
replace those strings with explanations appropriate to their actual use.
Android permissions are declared by the plugin and requested by the app at
runtime. Packaged Windows/AppContainer apps must declare their Bluetooth device
capability.
