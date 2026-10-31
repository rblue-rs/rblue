# rblue-corebluetooth error mapping

Every platform failure is mapped into the unified `rblue_api::Error` at the
backend boundary; raw CoreBluetooth `NSError` values never leak through the
public API. The forked btleplug flattens `NSError`s into string messages
(`Error::RuntimeError(String)`), so the mapping is partly structural
(btleplug error variants) and partly best-effort string normalization. The
normalization code lives in `src/mapping.rs`; this table is its contract.

## Structural mapping (btleplug variant -> rblue error)

| btleplug error | rblue error | Notes |
|---|---|---|
| `PermissionDenied` (fork pre-flight: `CBManager.authorizationClass` is denied/restricted) | `Error::Permission(Unauthorized)` | Raised from `initialize` |
| `NotConnected` | `Error::NotConnected` | |
| `TimedOut(duration)` | `Error::Timeout` | Fork default timeout helpers only |
| `DeviceNotFound` | `Error::NotFound(id)` | The backend already knows the queried id, so the variant is mapped at the call site to carry it |
| `RuntimeError(message)` | see string table below | |
| `NotSupported`, `NoAdapterAvailable`, `UnexpectedCallback`, `UnexpectedCharacteristic`, `NoSuchCharacteristic`, `Uuid`, `InvalidBDAddr`, `Other` | `Error::Backend(message)` | Fallback for everything without a structured status code |

## String normalization (`RuntimeError` message -> rblue error)

CoreBluetooth reports GATT failures through delegate callbacks carrying an
`NSError`; the fork forwards the localized description. Recognized families:

| Message family | rblue error | Origin |
|---|---|---|
| `"Device disconnected"` | `Error::Disconnected` | Fork drains pending futures with this message when the peripheral disappears mid-operation |
| `"... no longer available"` (`Peripheral` / `Service` / `Characteristic` no longer available) | `Error::Att(AttError { code: 0x01, handle: None })` | Stale session identifier: the fork removed the object after a disconnect or a service modification. Matches the mock's invalid-handle treatment of stale ids |
| `"(ATT error N.)"` / `"CBATTErrorDomain Code=N"` / `"CBATTErrorDomain error N"` | `Error::Att(AttError { code: N, handle: None })` | ATT-level GATT failure; the code is re-extracted (`parse_att_error_code`). CoreBluetooth does not report the failing attribute handle |
| anything else | `Error::Backend(message)` | Connection failures (`didFailToConnect`), descriptor errors, etc. |

Frequently seen ATT codes after normalization:

- `0x02` read not permitted, `0x03` write not permitted - the peripheral
  rejected the operation.
- `0x05` insufficient authentication / `0x0F` insufficient encryption - the
  encrypted-characteristic access that triggers iOS implicit pairing; the
  retry after the (automatic) pairing completes is the caller's move. This
  backend's `pair()` is `Error::Unsupported` by design.
- `0x08` insufficient authorization.
- `0x0D` invalid attribute value length - server-side rejection; distinct
  from the client-side `Error::InvalidLength` check below.
- `0x11` insufficient resources.

## Errors synthesized by the shell itself

| Situation | Error |
|---|---|
| Any operation before `initialize()` resolved the adapter state (`start_scan`, `stop_scan`) | `Error::InvalidState("adapter not initialized; call initialize() first")` |
| `initialize()` and the radio is powered off | `Error::Permission(BluetoothOff)` |
| `initialize()` and CoreBluetooth reports unsupported/resetting hardware (the fork collapses these into `CentralState::Unknown`) | `Error::Unsupported { platform: "corebluetooth", feature: "adapter" }` |
| No adapter could be created | `Error::Unsupported { platform: "corebluetooth", feature: "adapter" }` |
| `stop_scan` with no scan running | `Error::InvalidState("no scan is running")` |
| GATT operation (`read`/`write`/`subscribe`/`unsubscribe`/`max_write_len`/`discover_services`/`rssi`) while not connected | `Error::NotConnected` - checked before the platform call, so drained-future errors never surface for this case |
| Session id (`CharacteristicId`) that does not resolve against the discovered tree (never discovered, stale after reconnect or after `DeviceServicesModified`) | `Error::Att(AttError { code: 0x01, handle: None })` |
| `write` in a mode the property bits do not allow (`WithResponse` needs `WRITE`, `WithoutResponse` needs `WRITE_WITHOUT_RESPONSE`) | `Error::Att` code `0x03` (write not permitted) |
| `subscribe` on a characteristic with neither `NOTIFY` nor `INDICATE` | `Error::InvalidState` |
| Payload larger than `max_write_len` | `Error::InvalidLength { len, max }` - `len` is the exact payload length, `max` the exact `max_write_len` value. Never truncated, never chunked (native `writeValue` would silently truncate in the without-response mode) |
| `connect` on a device CoreBluetooth no longer knows (retrieval by identifier returned nothing) | `Error::NotFound(id)` |
| `peripheral(id)` for a device never sighted by this adapter (any id kind; CoreBluetooth ids are always `DeviceId::Uuid`) | `Error::NotFound(id)` |
| `mtu()` | `Error::Unsupported { platform: "corebluetooth", feature: "mtu" }` - the ATT MTU is system-managed; use `max_write_len` |
| `pair()` | `Error::Unsupported { platform: "corebluetooth", feature: "pairing" }` - iOS/macOS pair implicitly on encrypted-characteristic access |
| `request_device(opts)` | `Error::Unsupported { platform: "corebluetooth", feature: "request_device" }` - web-only entry point |

## Deliberately unused errors

`Error::Platform { code, message }` is never produced by this backend: the
fork exposes no numeric status channel, and CoreBluetooth error codes that
are not ATT-domain (connection failures, radio errors) arrive only as
localized strings, which are preserved verbatim in `Error::Backend`. If the
fork later forwards `NSError` codes structurally, connection-failure codes
should move to `Error::Platform` so supervisors can match them.
