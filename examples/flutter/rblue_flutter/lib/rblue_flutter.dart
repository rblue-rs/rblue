/// Early experimental Flutter binding for the rblue BLE project.
///
/// The package wraps the message-passing `rblue-c` ABI. [RblueClient] maps
/// request IDs to Dart futures and exposes adapter events and notifications as
/// broadcast streams. Native libraries for Android, iOS, macOS, and Windows
/// are bundled in the package; see the README for the exact support matrix and
/// known limitations.
///
/// This API is a development preview and may change before a stable release.
library;

export 'src/client.dart';
export 'src/models.dart';
export 'src/plugin_channel.dart';
