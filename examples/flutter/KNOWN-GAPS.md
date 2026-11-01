# Flutter example: known gaps

These are limitations of the example or its current shared FFI surface, not
silent claims of platform support.

1. **No simulator radio coverage.** Android and iOS simulators can validate
   packaging/UI only; Bluetooth behavior requires physical devices.
2. **Reconnect lives in Dart.** The Rust `ConnectionSupervisor` and GATT queue
   are not exported through `rblue-c`. The example intentionally demonstrates
   host-side reconnect; after a successful retry an application must rediscover
   services and restore subscriptions.
3. **Notification backpressure is host-managed.** The C callback is copied into
   a Dart broadcast stream through `NativeCallable.listener`; the stream has no
   built-in per-listener drop counter. High-rate production consumers should
   add an explicit bounded aggregation policy.
4. **Generated ids are connection-scoped.** `CharacteristicId` values are
   rebuilt with instance zero from the current tree. Devices with repeated
   same-UUID service/characteristic instances need an API that exposes instance
   coordinates directly in discovery results.
5. **No release signing.** CI builds Android debug and iOS no-codesign output.
   Store signing, provisioning profiles, and distribution are application
   concerns.
6. **Android status loss inherited from the btleplug fork.** Operation callback
   status values such as 133 are preserved when present, but some connection
   paths in the fork retry and collapse their final failure to `NotConnected`.
7. **The C ABI is intentionally small.** `rssi`, `mtu`, pairing, and explicit
   `max_write_len` requests are not yet exposed by `rblue-c`; the sample focuses
   on scan/connect/discover/read/write/subscribe.
