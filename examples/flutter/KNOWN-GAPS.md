# Flutter example: known gaps

These are limitations of the example or its current shared FFI surface, not
silent claims of platform support.

1. **No simulator radio coverage.** Android and iOS simulators can validate
   packaging/UI only; Bluetooth behavior requires physical devices.
2. **Reconnect lives in Dart.** The Rust `ConnectionSupervisor` and GATT queue
   are not exported through `rblue-c`. The example intentionally demonstrates
   host-side reconnect; it rediscovers services and optionally restores the
   subscriptions captured when monitoring starts. Matching fresh instances
   still needs physical-device acceptance on repeated-UUID peripherals.
3. **Notification backpressure is host-managed.** The C callback is copied into
   a Dart broadcast stream through `NativeCallable.listener`; the stream has no
   built-in per-listener drop counter. High-rate production consumers should
   add an explicit bounded aggregation policy.
4. **Generated ids are connection-scoped.** Dart derives instance coordinates
   from discovery order. Backend representations may collapse repeated UUIDs;
   end-to-end behavior for such devices still requires physical validation.
5. **No release signing.** CI builds Android debug and iOS no-codesign output.
   Store signing, provisioning profiles, and distribution are application
   concerns.
6. **Android status loss inherited from the btleplug fork.** Operation callback
   status values such as 133 are preserved when present, but some connection
   paths in the fork retry and collapse their final failure to `NotConnected`.
7. **The C ABI is intentionally small.** RSSI, MTU query, capabilities and
   `max_write_len`, and descriptor `read_descriptor` / `write_descriptor` are
   exposed. Explicit MTU requests and pairing remain platform-specific.
