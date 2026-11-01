/// Local Flutter plugin for the rblue BLE example.
///
/// The plugin wraps the message-passing C ABI of `crates/rblue-c`:
/// `RblueClient` turns `rblue_request` calls into typed Dart futures
/// (req_id -> Completer) and the two C callbacks into broadcast streams.
///
/// See the plugin README for the initialization discipline (the Android JNI
/// step) and `../tools/build-libs.sh` for the prebuilt library workflow.
library;

export 'src/client.dart';
export 'src/models.dart';
export 'src/plugin_channel.dart';
