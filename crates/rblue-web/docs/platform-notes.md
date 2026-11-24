# Web platform notes (rblue-web)

Platform facts and constraints for the `wasm32` Web Bluetooth backend. The
API-to-browser mapping table lives in `docs/error-mapping.md`.

## Browser support

- Web Bluetooth is implemented in Chromium browsers (Chrome, Edge, Opera)
  only. Safari and Firefox do not expose `navigator.bluetooth`.
- Where `navigator.bluetooth` is missing, `default_adapter()` still
  constructs an adapter, but `initialize()` - and every other entry point -
  returns `Error::Unsupported { platform: "web", feature: "bluetooth" }`.
- Chrome requires a **secure context**: `https://` (or `localhost` for
  development). On insecure origins `requestDevice` fails with a
  `SecurityError`.

## Device selection: no background scan

- The browser never scans on its own. `start_scan`/`stop_scan` return
  `Error::Unsupported { platform: "web", feature: "scan" }`, and
  `capabilities()` reports neither `ActiveScan` nor `PassiveScan`.
- Devices are obtained exclusively through `request_device()`, which pops the
  browser chooser (`RequestDevicePicker` capability).
- `request_device()` **must run inside a user gesture** (a click/touch
  handler). The Rust core cannot detect the calling context; the facade layer
  (the TS/npm package) is responsible for exposing a
  `requestDeviceFromClick(handler)`-style helper and marking the call site.
  Called outside a gesture, the browser raises `SecurityError`, mapped to
  `Error::Permission(Unauthorized)`.
- `RequestDeviceOptions.services` are merged into `optionalServices`
  automatically together with the caller-supplied `optional_services`. A
  service absent from `optionalServices` at chooser time makes the browser
  raise `SecurityError` on first access, **even if it was used to select the
  device** - this is the single most common Web Bluetooth integration trap.

## Writes, MTU, chunking

- The browser does **not** chunk writes, and the negotiated ATT MTU is not
  observable. Per the unified write semantics, `write` rejects payloads above
  20 bytes (default ATT MTU 23 minus 3) with `Error::InvalidLength { len,
  max }` before any browser call - values are never silently truncated and
  never hiddenly chunked.
- `max_write_len()` reports the same conservative 20 for both write modes;
  `mtu()` returns `Error::Unsupported`. When a link actually negotiates a
  larger MTU, the extra capacity is unavailable until the platform grows an
  API for it - callers must not rely on more than 20 bytes.
- Chunking long transfers is the caller's/facade's job: the TS layer should
  expose an explicit helper (e.g. `writeLong(characteristic, data, chunkSize
  = 20)`), honoring `WriteMode` and the characteristic properties. rblue
  deliberately does not hide chunking behind an automatic loop.

## Connections and disconnects

- `connect()` maps to `device.gatt().connect()`; it is idempotent for an
  already-connected device.
- `discover_services()` maps to `getPrimaryServices()`; the browser
  permission-filters the result against `optionalServices` before it reaches
  Rust. Descriptor enumeration is not performed (Phase 2).
- The `gattserverdisconnected` event is translated into
  `AdapterEvent::DeviceDisconnected` on the adapter event stream (reason
  `Local` after an explicit `disconnect()`, otherwise `Unknown`).
- **The web does not reconnect by itself.** After a disconnect the supervisor
  must call `connect()` again; the GATT tree must then be re-discovered and
  subscriptions re-established. `CharacteristicId`s from the previous
  connection are invalid (operations on them fail with a browser error
  mapped to `Disconnected`).
- Notifications flow via `startNotifications()` plus a
  `characteristicvaluechanged` listener; notify and indicate are
  browser-internal and both arrive through the same event. Values are
  broadcast to every `notifications()` subscriber.

## Capabilities

| Capability | Web |
|---|---|
| `GattClient` | yes |
| `RequestDevicePicker` | yes |
| `WatchAdvertisements` | runtime probe only (`window.BluetoothDevice.prototype`, Chrome flag); absent until detected - see `rblue_web::detect_watch_advertisements()` |
| `ActiveScan` / `PassiveScan` / `ScanResponse` / `BackgroundScan` | no (no browser scanning) |
| `MtuNegotiation` / `MtuQuery` | no (MTU not observable) |
| `RssiDuringConnection` | no (`rssi()` unsupported) |
| `Pairing` | no (`pair()` / `unpair()` unsupported with feature `pair` / `unpair`; the browser pairs implicitly when an encrypted characteristic is accessed) |
| `GattServer` / `Advertise` | no | Web Bluetooth has no peripheral role; the methods return `Error::Unsupported` (issue #49) |
| `ConnectionPriority` | no (`set_connection_priority` returns `Error::Unsupported`; Web Bluetooth has no equivalent API) |
| `SystemConnectedPeripherals` | no (`system_connected_peripherals` returns `Error::Unsupported`; Web Bluetooth has no equivalent lookup) |

## Build and runtime notes

- **Binding strategy**: `web-sys` gates its Web Bluetooth bindings behind
  `--cfg=web_sys_unstable_apis` (a `RUSTFLAGS` requirement that would leak
  into every build of this crate and its dependents). rblue-web declares the
  small Web Bluetooth DOM surface it needs directly with `wasm_bindgen` and
  uses `web-sys` only for the stable `Window`/`Navigator`/`Event`/
  `EventTarget` pieces. Plain
  `cargo check -p rblue-web --target wasm32-unknown-unknown` needs no special
  flags.
- **Single-threaded realm**: Web Bluetooth is main-thread-only. JS values are
  wrapped in an internal `Send`/`Sync` type to satisfy the runtime-neutral
  trait bounds; never move an adapter into a Web Worker - construct one
  inside the worker's realm instead.
- **Identifier privacy**: `BluetoothDevice.id` is an opaque, per-origin,
  long-lived string. Treat it as personal data; do not log it at info level
  or above (same rule as MAC addresses elsewhere in rblue).
- **Snapshot semantics**: the adapter event stream replays known devices as
  fresh `DeviceDiscovered` events to new subscribers; on the web the only
  known advertisement field is the advertised local name captured at chooser
  time.

## Testing status / TODO

- Pure logic (DOM-exception mapping, request-device option assembly, id
  wrapping, the bounded broadcast bus) is unit-tested on the host
  (`cargo test -p rblue-web`).
- Browser end-to-end tests (`wasm-bindgen-test` + headless Chrome with
  `--enable-features=WebBluetooth`, and CDP-based `requestDevice`
  interception to fake devices, per the implementation plan's testing
  strategy) are **not part of this phase** and remain TODO for the Web
  milestone.
