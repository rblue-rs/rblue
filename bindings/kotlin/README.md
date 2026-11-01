# bindings/kotlin - UniFFI Kotlin packaging project

Android AAR for the rblue UniFFI facade (`crates/rblue-ffi`). Consumed by the
Compose example under `examples/android` (module `:rblue`), and reusable by any
Kotlin application that wants BLE through rblue.

## Layout

| Path | Contents |
|---|---|
| `generated/` | Canonical `uniffi-bindgen --language kotlin` output, committed for review (same policy as `bindings/swift/generated`). Regenerate with `examples/android/scripts/build-bindings.sh`; CI rejects drift. |
| `rblue/` | The AAR library module (`com.rblue`). Wraps the generated bindings with `RblueAndroid.initialize(context)`. |
| `rblue/src/real/jniLibs/` | `librblue_ffi.so` per ABI, built against the real Android backend. Build artifacts; gitignored. |
| `rblue/src/mock/jniLibs/` | `librblue_ffi.so` for `arm64-v8a`/`x86_64` built against the mock backend (no Bluetooth hardware needed; runs on emulators). Gitignored. |
| `btleplug-java/` | Java helper classes vendored from the btleplug fork (`com.nonpolynomial.btleplug.android.impl` + `io.github.gedgygedgy.rust.*`). Hard runtime dependency of the Android backend; see `NOTICE.md`. |

## Module coordinates

- `:rblue` - namespace `com.rblue`. The `real` product flavor ships the JNI
  backend; the `mock` flavor ships a mock-backend `.so` and skips JNI init, so
  the app UI runs without any Bluetooth hardware.
- `:btleplug-java` - the fork's Java helper classes (BSD-3, see `NOTICE.md`).

## Build

Run from `examples/android/` (the Gradle root):

```sh
# one-time: produce the .so files and the generated bindings
scripts/build-bindings.sh

# then any Gradle invocation, e.g.
JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew :app:assembleRealDebug
```

`scripts/build-bindings.sh` documents its tool requirements (Android SDK + NDK,
rust android targets). Generated Kotlin is regenerated into
`bindings/kotlin/generated` on every run.

## Runtime discipline (mirrors crates/rblue-android/docs/platform-notes.md)

- Call `RblueAndroid.initialize(context)` once, from the application process,
  before constructing `uniffi.rblue_ffi.Rblue()`. It bridges into
  `com.rblue.RblueNative.nativeInit()` (exported by `librblue_ffi.so`) on the
  calling thread, which must be an application thread that sees the
  application `ClassLoader` - the fork resolves its helper classes through it.
- Never call any of this from a `static` initializer: the Android ClassLoader
  is not ready there.
- The call is idempotent; the fork's own init is guarded.
