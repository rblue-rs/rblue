# rblue-winrt error mapping

Every platform failure is mapped into the unified `rblue_api::Error` at the
backend boundary; raw WinRT `HRESULT`s and `GattCommunicationStatus` values
never leak through the public API as opaque strings. The forked btleplug
flattens Windows failures into two string shapes - the Debug form of the raw
`GattCommunicationStatus(N)` status value, and the Debug form of a thrown
WinRT operation (`windows::core::Error`), which carries `HRESULT(0x...)` -
so the mapping re-extracts the structured codes instead of losing them. The
normalization code lives in `src/mapping.rs`; this table is its contract.

## Mapping precedence

Shell-synthesized checks run before any platform call, so platform errors
only surface for genuinely failing platform operations. Per GATT operation
the order is:

1. Connection check (`Error::NotConnected` when the link is down).
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
| `PermissionDenied` (GATT status `AccessDenied`) | `Error::Permission(Unauthorized)` | Raised on operations the OS access policy rejects |
| `NotConnected` (GATT status `Unreachable`) | `Error::NotConnected` | The connect path reports `Unreachable` through this variant |
| `TimedOut(duration)` | `Error::Timeout` | Fork timeout helpers only |
| `DeviceNotFound` | `Error::NotFound(id)` on `connect`; `Error::Backend` elsewhere | `connect` knows the queried id and maps the variant at the call site: `FromBluetoothAddressAsync` could not resolve the address (see the RPA note in platform-notes.md). Every other path keeps the generic fallback |
| `NotSupported(message)` | see string table below | |
| `RuntimeError(message)` | see string table below | winrtble does not author these today; normalized for future-proofing |
| `Other(source)` | see string table below | All thrown WinRT operations land here |
| `NoAdapterAvailable`, `UnexpectedCallback`, `UnexpectedCharacteristic`, `NoSuchCharacteristic`, `Uuid`, `InvalidBDAddr` | `Error::Backend(message)` | Fallback for everything without a structured status code |

## String normalization (fork error strings -> rblue error)

### GATT status shape: `GattCommunicationStatus(N)`

Raw Windows `GattCommunicationStatus` values (Success=0, Unreachable=1,
ProtocolError=2, AccessDenied=3):

| Status | rblue error |
|---|---|
| `Unreachable` (1) | `Error::NotConnected` |
| `AccessDenied` (3) | `Error::Permission(Unauthorized)` |
| `ProtocolError` (2) | `Error::Platform { code: 2, message }` - the peer returned an ATT error response, but this status alone does not carry the ATT code, so the raw Windows status value is preserved for supervisors |
| any other value | `Error::Platform { code: N, message }` - future SDK values keep their numeric code |

The bare string `"ProtocolError"` (the fork's `NotSupported` rendering of the
same status) maps identically to `Error::Platform { code: 2 }`.

### HRESULT shape: `HRESULT(0x...)`

Thrown WinRT operations, extracted from the `windows::core::Error` Debug
string:

| HRESULT | rblue error | Notes |
|---|---|---|
| `0x80620010` | `Error::NotConnected` | The device-unreachable code Windows raises on the winrtble path when the peripheral cannot be reached (connect or GATT operation on a device that is gone). Plan section 2.2 mandate |
| `0x8062000A` | `Error::Timeout` | Expired connection attempt. Plan section 2.2 mandate |
| `0x80070005` (`E_ACCESSDENIED`) | `Error::Permission(Unauthorized)` | |
| `0x806200XX` (other, `XX != 0`) | `Error::Att(AttError { code: XX, handle: None })` | `E_BLUETOOTH_ATT_*` family: the low byte is the ATT protocol error code (e.g. `0x80620003` write not permitted, `0x8062000D` invalid attribute value length). WinRT does not report the failing attribute handle |
| anything else | `Error::Platform { code, message }` | The numeric code is preserved verbatim; never demoted to `Backend` |

### Everything else

| Message | rblue error | Origin |
|---|---|---|
| `"Service not found for ..."`, `"Characteristic not found for ..."`, `"Descriptor not found for ..."` | `Error::Att(AttError { code: 0x01, handle: None })` | The fork's per-UUID service/characteristic maps lack the requested UUID: a stale or fabricated session identifier (the fork rebuilds its maps per connection). Matches the mock's invalid-handle treatment |
| `"Can not subscribe to attribute"` | `Error::Backend` | Unreachable in practice: the shell pre-checks NOTIFY/INDICATE |
| anything else (`"Communication Error:"` - the fork's fallback for unknown GATT statuses, dropped their numeric value) | `Error::Backend(message)` | No structured code survived the fork |

Frequently seen ATT codes after normalization: `0x02` read not permitted,
`0x03` write not permitted, `0x05` insufficient authentication,
`0x08` insufficient authorization, `0x0D` invalid attribute value length
(server-side rejection; distinct from the client-side `Error::InvalidLength`
check), `0x0F` insufficient encryption, `0x11` insufficient resources.

## Errors synthesized by the shell itself

| Situation | Error |
|---|---|
| Any operation before `initialize()` resolved the adapter state (`start_scan`, `stop_scan`) | `Error::InvalidState("adapter not initialized; call initialize() first")` |
| `initialize()` and the radio is off or disabled (the fork maps `RadioState::Off` to `PoweredOff` and collapses `Disabled`/`NotAvailable`/`Unknown` into `CentralState::Unknown`; the common cause of both is the user's Bluetooth toggle) | `Error::Permission(BluetoothOff)` |
| `initialize()` and no Bluetooth radio exists (empty adapter enumeration) | `Error::Unsupported { platform: "winrt", feature: "adapter" }` |
| `stop_scan` with no scan running | `Error::InvalidState("no scan is running")` |
| GATT operation (`read`/`write`/`subscribe`/`unsubscribe`/`max_write_len`/`discover_services`/`mtu`) while not connected | `Error::NotConnected` - checked before the platform call |
| Session id (`CharacteristicId`) that does not resolve against the discovered tree (never discovered, stale after reconnect; the tree is cleared on disconnect) | `Error::Att(AttError { code: 0x01, handle: None })` |
| `write` in a mode the property bits do not allow (`WithResponse` needs `WRITE`, `WithoutResponse` needs `WRITE_WITHOUT_RESPONSE`) | `Error::Att` code `0x03` (write not permitted) |
| `subscribe` on a characteristic with neither `NOTIFY` nor `INDICATE` | `Error::InvalidState` |
| Payload larger than `max_write_len` | `Error::InvalidLength { len, max }` - `len` is the exact payload length, `max` the exact `max_write_len` value. Never truncated, never chunked |
| `connect` on a device whose address the OS does not know in this boot session (`FromBluetoothAddressAsync` failed) | `Error::NotFound(id)` - Windows can only connect to watcher-observed addresses (RPA rotation included); re-scan first. See platform-notes.md |
| `peripheral(id)` for a device never sighted by this adapter (any id kind; Windows ids are always `DeviceId::Mac`) | `Error::NotFound(id)` |
| `rssi()` | `Error::Unsupported { platform: "winrt", feature: "rssi" }` - Windows has no connected-RSSI API; the fork would return the last scan-time value, which the shell refuses to present as a connected reading |
| `pair()` | `Error::Unsupported { platform: "winrt", feature: "pair" }` - WinRT has `PairAsync` but the fork exposes no API; reported as absent rather than faked |
| `unpair()` | `Error::Unsupported { platform: "winrt", feature: "unpair" }` |
| `request_device(opts)` | `Error::Unsupported { platform: "winrt", feature: "request_device" }` - web-only entry point |

## Deliberate notes

- **Mid-operation link loss surfaces as `Error::NotConnected`**, not
  `Error::Disconnected`: the unreachable-device status/HRESULT that Windows
  raises for a just-dropped link is indistinguishable from an operation
  started on a dead link. Supervisors should drive reconnects from the
  `DeviceDisconnected` adapter event, not from the operation error.
- **Discovery failures may surface late.** The fork's
  `discover_services` returns `Ok` even when the GATT service fetch reports
  a non-success status (the tree is then empty), so a failed discovery can
  appear as an empty tree and the failure surfaces later as an
  invalid-handle `Error::Att` on the first characteristic operation. This is
  an inherited fork limitation; the shell does not invent errors to mask it.
- `Error::Disconnected` is never produced by this backend for the reason
  above; `Error::Pairing` and `Error::Att` with a known handle are also
  never produced (WinRT does not report the failing attribute handle, and
  active pairing is unavailable through the fork).
