# rblue-bluez error mapping

Every platform failure is mapped into the unified `rblue_api::Error` at the
backend boundary; raw D-Bus error names never leak through the public API as
opaque strings. Unlike Windows/Android, BlueZ's D-Bus surface carries no
numeric status codes - failures are identified by an error *name*
(`org.bluez.Error.*`) plus a human-readable message, so this backend's
structured-code preservation takes the form of a name table: names with a
unified equivalent map to it, and peer ATT errors are re-derived from the
name + message pair bluetoothd produces (`btd_error_att_code`). Because no
numeric codes exist on this platform, `Error::Platform` is never produced;
failures without a semantic mapping surface as `Error::Backend` with the
verbatim `name: message` text preserved. The normalization code lives in
`src/mapping.rs`; this table is its contract.

## Mapping precedence

Shell-synthesized checks run before any platform call, so platform errors
only surface for genuinely failing platform operations. Per GATT operation
the order is:

1. Connection check (`Error::NotConnected` when the link is down; the
   registry entry carries no live handle on `connect` -> `Error::NotFound`).
2. Property-bit check (`Error::Att` code `0x03` when the write mode does not
   match the characteristic properties) / subscribe property check
   (`Error::InvalidState` when the characteristic has neither `NOTIFY` nor
   `INDICATE`).
3. Session-identifier resolution against the discovered tree
   (`Error::Att` invalid handle when it does not resolve).
4. Payload-length check (`Error::InvalidLength { len, max }` with `max`
   exactly the `max_write_len` value).
5. The platform call, whose failures go through the tables below.

## Structural mapping (btleplug variant -> rblue error)

| btleplug error | rblue error | Notes |
|---|---|---|
| `NotConnected` | `Error::NotConnected` | Authored by the fork's `read_rssi` on a device without RSSI; the shell does not call it, mapped for completeness |
| `TimedOut(duration)` | `Error::Timeout` | Not authored by the bluez fork today (its timeouts surface as `ServiceDiscoveryTimedOut`); normalized for future-proofing |
| `Other(source)` | see tables below | All bluez-async failures (`Box<bluez_async::BluetoothError>`) and fork-authored string errors land here |
| `DeviceNotFound` | `Error::NotFound(id)` at the `Adapter::peripheral` call site; `Error::Backend` elsewhere | The fork raises it when `GetManagedObjects`/`Get` cannot resolve the device object path; the call site knows the queried id |
| `NoAdapterAvailable`, `PermissionDenied`, `UnexpectedCallback`, `UnexpectedCharacteristic`, `NoSuchCharacteristic`, `NotSupported`, `RuntimeError`, `Uuid`, `InvalidBDAddr` | `Error::Backend(message)` | Fallback for everything without a structured D-Bus name (`PermissionDenied` is winrtble-only; the bluez fork's poisoned-lock failures surface as `Other` strings) |

## bluez-async mapping (`BluetoothError` -> rblue error)

| `BluetoothError` variant | rblue error | Notes |
|---|---|---|
| `NoBluetoothAdapters` | `Error::Unsupported { platform: "bluez", feature: "adapter" }` | `GetManagedObjects` returned no adapter objects: no radio present. Reached through the shell's adapter enumeration |
| `DbusError(error)` | see D-Bus name table below | The main path |
| `ServiceDiscoveryTimedOut` | `Error::Timeout` | The fork's `connect` awaits service resolution with a 30 s deadline (`await_service_discovery`) |
| `UuidNotFound { .. }` | `Error::Att` code `0x01` (invalid handle) | bluez-async's by-UUID lookups; a stale or fabricated session identifier, same treatment as the fork's own UUID lookups |
| `XmlParseError`, `UuidParseError`, `FlagParseError`, `AddressTypeParseError`, `RequiredPropertyMissing`, `MacAddressParseError`, `ModaliasParseError`, `DeviceIdParseError` | `Error::Backend(message)` | Property/introspection parsing failures; no structured code to preserve beyond the message |

## D-Bus name table (`org.bluez.Error.*` -> rblue error)

bluetoothd reports peer ATT errors either under a dedicated D-Bus name or
under `org.bluez.Error.Failed` with the ATT error description as the message
(`src/error.c`, `btd_error_att_code`). The table reads both. The
`AttError.handle` field is always `None`: D-Bus does not carry the failing
attribute handle.

| D-Bus name | message | rblue error |
|---|---|---|
| `org.bluez.Error.NotConnected` | - | `Error::NotConnected` |
| `org.bluez.Error.NotPermitted` | `Read Not Permitted` | `Error::Att` code `0x02` |
| `org.bluez.Error.NotPermitted` | `Write Not Permitted` | `Error::Att` code `0x03` |
| `org.bluez.Error.NotPermitted` | anything else | `Error::Backend` with name + message (non-ATT rejections keep their text) |
| `org.bluez.Error.NotAuthorized` | `Insufficient Authentication` | `Error::Att` code `0x05` |
| `org.bluez.Error.NotAuthorized` | `Insufficient Authorization` | `Error::Att` code `0x08` |
| `org.bluez.Error.NotAuthorized` | `Insufficient Key Size` | `Error::Att` code `0x0C` |
| `org.bluez.Error.NotAuthorized` | `Insufficient Encryption` | `Error::Att` code `0x0F` |
| `org.bluez.Error.NotAuthorized` | anything else | `Error::Backend` with name + message |
| `org.bluez.Error.NotSupported` | `Unsupported Request` | `Error::Att` code `0x06` |
| `org.bluez.Error.NotSupported` | anything else | `Error::Backend` with name + message |
| `org.bluez.Error.InvalidOffset` | - | `Error::Att` code `0x07` |
| `org.bluez.Error.InvalidLength` | - | `Error::Att` code `0x0D` (oversize payloads are rejected by the shell *before* the call, so this is the peer's verdict) |
| `org.bluez.Error.DoesNotExist` | `Attribute Not Found` | `Error::Att` code `0x0A` |
| `org.bluez.Error.DoesNotExist` | anything else | `Error::Backend` with name + message (unknown object path, e.g. the device object vanished mid-operation) |
| `org.bluez.Error.Failed` | `Invalid Handle` | `Error::Att` code `0x01` |
| `org.bluez.Error.Failed` | `Invalid PDU` | `Error::Att` code `0x04` |
| `org.bluez.Error.Failed` | `Prepare Queue Full` | `Error::Att` code `0x09` |
| `org.bluez.Error.Failed` | `Attribute Not Long` | `Error::Att` code `0x0B` |
| `org.bluez.Error.Failed` | `Unlikely Error` | `Error::Att` code `0x0E` |
| `org.bluez.Error.Failed` | `Insufficient Resources` | `Error::Att` code `0x11` |
| `org.bluez.Error.Failed` | anything else | `Error::Backend` with name + message (the generic BlueZ failure: daemon-side aborts, I/O errors, profile failures) |
| `org.bluez.Error.Canceled` | - | `Error::Disconnected` - BlueZ cancels in-flight operations when the link drops |
| `org.bluez.Error.InProgress` | - | `Error::InvalidState` - an operation of the same kind is already running |
| `org.bluez.Error.NotReady` | - | `Error::Permission(BluetoothOff)` - the adapter is powered down (or bluetoothd is still starting); the unified model's expected "radio off" state |
| `org.bluez.Error.AlreadyConnected`, `org.bluez.Error.AlreadyExists` | - | success on the `connect` call site only: BlueZ raises these for `Device1.Connect()` on an already-connected device, and the shell's `is_connected` pre-check narrows the window to a concurrent connection whose result is what the caller asked for. In the general table they fall through to `Error::Backend` |
| any other name (`org.freedesktop.DBus.Error.*` included) | - | `Error::Backend` with the verbatim `name: message` - e.g. `org.freedesktop.DBus.Error.ServiceUnknown` ("The name org.bluez was not provided by any .service files") means bluetoothd is not running; that is a deployment failure, not an expected capability gap, so it stays a backend error rather than `Error::Unsupported` |
| no name | - | `Error::Backend("dbus error without a name: {message}")` |

Message matching is case-insensitive against bluetoothd's fixed description
strings; a message that does not match a known description stays unstructured
(`Error::Backend`) instead of being guessed into an ATT code.

## Fork string errors (`Other` payload is a plain string)

| message shape | rblue error | Notes |
|---|---|---|
| `Service with UUID {uuid} not found.` / `Characteristic with UUID {uuid} not found.` / `Descriptor with UUID {uuid} not found.` | `Error::Att` code `0x01` (invalid handle) | The fork's per-UUID maps lack the requested UUID - a stale or fabricated session identifier (the fork rebuilds its per-connection GATT maps on every discovery, and collapses same-UUID entries) |
| anything else (poisoned-lock `PoisonError` debug text, future fork strings) | `Error::Backend` with the message | No structured code left to preserve |

## Errors synthesized by the shell itself

| situation | rblue error |
|---|---|
| trait method called before `initialize` | `Error::InvalidState("adapter not initialized; call initialize() first")` |
| `stop_scan` with no scan running | `Error::InvalidState("no scan is running")` |
| `peripheral(id)` with a `DeviceId` that is not `DeviceId::Mac`, or an address never recorded in this adapter instance's registry | `Error::NotFound(id)` carrying the queried id (BlueZ itself could connect to cache-known devices, but the shell's registry is the contract source; `initialize` imports the BlueZ cache, so anything bluetoothd knows is normally present) |
| `request_device` | `Error::Unsupported { platform: "bluez", feature: "request_device" }` |
| `pair` | `Error::Unsupported { platform: "bluez", feature: "pairing" }` |
| `rssi` on a connected device | `Error::Unsupported { platform: "bluez", feature: "rssi" }` |
| `initialize` with no adapter objects on the bus | `Error::Unsupported { platform: "bluez", feature: "adapter" }` |

## Deliberate notes

- **No numeric status codes exist on this platform.** `Error::Platform`
  exists for platforms whose failures carry raw numbers (Android GATT
  status, OHOS error codes); D-Bus failures carry names, so the name table
  above is this backend's equivalent of structured-code preservation, and
  unmapped failures keep their verbatim `name: message` text in
  `Error::Backend`.
- **`Error::Disconnected` only surfaces from `org.bluez.Error.Canceled`.**
  A disconnect that is not racing an in-flight operation is delivered as a
  `DeviceDisconnected` *event* (with `DisconnectReason::Unknown` - BlueZ's
  D-Bus API reports no structured reason), not as an operation error.
- **Idempotency is shell-provided, not platform-provided.** `disconnect`
  pre-checks the connection state (BlueZ's `Disconnect()` rejects with
  `NotConnected` otherwise), `unsubscribe` relies on the fork mapping the
  "No notify session started" rejection to success, and `connect` treats the
  already-connected rejections as success (see the D-Bus table).
- The ATT-code recovery from `Failed`/`NotPermitted`/`NotAuthorized`
  messages matches bluetoothd's stable description strings. If a future
  BlueZ changes the wording, the affected failures degrade to
  `Error::Backend` with the verbatim text - never to a wrong ATT code.
