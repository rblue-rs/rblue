# rblue-android error mapping

Every platform failure is mapped into the unified `rblue_api::Error` at the
backend boundary; raw JNI/GATT values never leak through the public API as
opaque strings. This platform is the reason `Error::Platform { code, message }`
exists: Android GATT callbacks carry numeric status integers, and the
connection supervisor matches the raw code (133 above all - Android's generic
"the GATT stack gave up" failure and the leading connect-failure cause) instead
of re-parsing text. **The hard rule: every numeric status that reaches Rust
becomes `Error::Platform` verbatim; it is never demoted into
`Error::Backend`.** The normalization code lives in `src/mapping.rs`; this
table is its contract.

## How status codes travel through the fork (point-by-point)

The fork's Java layer (`com.nonpolynomial.btleplug.android.impl.Peripheral`)
reports every failed GATT callback as a `RuntimeException` whose *message
text* embeds the raw status integer (`"Unable to read characteristic, status:
N"`); its Rust side surfaces those as `btleplug::Error::RuntimeError`. The
shell recovers the number from the stable `status: N` suffix. The transit
points, verified against the fork source:

| fork path | raw status fate |
|---|---|
| `connect` - `onConnectionStateChange` reports a failure | **Swallowed by the fork's Java layer.** It retries the fast-failing status 133 / HCI 0x3E attempts itself (up to 3 total attempts, only when the attempt failed quickly) and then throws `NotConnectedException` without the number. `connect()` therefore fails with `Error::NotConnected`, not `Platform{133}` (fork limitation, below). |
| `connect` - the fork's automatic `requestMtu(517)` after the link comes up | Preserved: `"MTU negotiation failed, status: N"` -> `Error::Platform { code: N }`. |
| `disconnect` callback failure | Preserved: `"Unable to disconnect, status: N"` -> `Error::Platform { code: N }`. |
| `discover_services` callback failure | Preserved: `"Unable to discover services, status: N"` -> `Error::Platform { code: N }`. |
| `read` / `write` callback failure | Preserved: `"Unable to read characteristic, status: N"` / `"Unable to write characteristic, status: N"` -> `Error::Platform { code: N }`. |
| `subscribe` / `unsubscribe` (CCCD descriptor write) failure | Preserved: `"Unable to write client characteristic configuration descriptor, status: N"` -> `Error::Platform { code: N }`. |
| `rssi` (`readRemoteRssi`) failure | Preserved: `"RSSI read failed, status: N"` -> `Error::Platform { code: N }`. |
| disconnect *event* | **No status exists on this path.** The fork's JNI disconnect notification carries only the address; the shell publishes `DeviceDisconnected(id, Some(DisconnectReason::Unknown))` (fork limitation, below). |
| `start_scan` permission denial | `SecurityException.getMessage()` text (no numeric code exists for it); classified into `Error::Permission` - see the permission table below. |
| scan callback failures (`onScanFailed`) | **Dropped inside the fork**: its `ScanCallback` does not override `onScanFailed`, so radio-off-mid-scan and scan-registration failures are silent (fork limitation, below). |

A consequence for the supervisor: the 133-specific backoff engages where the
number survives (GATT-operation failures, MTU-negotiation failures at
connect); a plain failed connect arrives as `Error::NotConnected` after the
fork's own internal 133 retries, and disconnect events carry no reason - the
supervisor's generic `NotConnected`/disconnect-driven backoff covers those.

## Android GATT status table (all values -> `Error::Platform` verbatim)

The mapping is a passthrough: every `status: N` suffix maps to
`Error::Platform { code: N, message: <full fork message> }` with no
reinterpretation, so statuses added by future Android versions are preserved
too. The table documents the well-known values for supervision policy
(partially platform constants of `android.bluetooth.BluetoothGatt`, partially
HCI error numbers the stack surfaces through the same channel):

| status | meaning | Notes |
|---|---|---|
| 0 | `GATT_SUCCESS` | Never appears in a failure message in practice (the fork throws only when status != 0); preserved as-is if it ever does |
| 1 | `GATT_INVALID_HANDLE` (ATT 0x01) | Peer ATT error |
| 2 | `GATT_READ_NOT_PERMITTED` (ATT 0x02) | Peer ATT error |
| 3 | `GATT_WRITE_NOT_PERMITTED` (ATT 0x03) | Peer ATT error - the shell rejects mode-mismatched writes before the call with `Error::Att` 0x03, so this arrives only from the peer |
| 4 | `GATT_INVALID_PDU` (ATT 0x04) | Peer ATT error |
| 5 | `GATT_INSUFFICIENT_AUTHENTICATION` (ATT 0x05) | Peer ATT error; pairing will be triggered by the stack for bonded-capable devices |
| 6 | `GATT_REQUEST_NOT_SUPPORTED` (ATT 0x06) | Peer ATT error |
| 7 | `GATT_INVALID_OFFSET` (ATT 0x07) | Peer ATT error |
| 8 | `GATT_INSUFFICIENT_AUTHORIZATION` | Peer ATT error slot reused by Android for authorization |
| 13 | `GATT_INVALID_ATTRIBUTE_LENGTH` (ATT 0x0D) | Peer ATT error - oversize payloads are rejected by the shell *before* the call, so this is the peer's verdict |
| 19 | HCI 0x13 - remote user terminated connection | The peer dropped the link; in-flight operations fail with this status |
| 22 | HCI 0x16 - connection terminated by local host | The local stack closed the link |
| 34 | HCI 0x22 - LMP/transaction timeout | Stack-level timeout while talking to the controller |
| 62 | HCI 0x3E - connection failed to establish | Companion of 133 for failed connects; the fork's Java layer retries this like 133 |
| **133** | `GATT_ERROR` - Android's generic GATT failure | **The headline code.** The leading connect-failure cause (link-layer establishment failures surface as this generic value), and a common failure for GATT operations on congested stacks. Must reach the supervisor as `Error::Platform { code: 133 }` / be absent where the fork swallows it (see the transit table) - never a string |
| 143 | `GATT_CONNECTION_CONGESTION` (0x8F) | Transient; retryable per policy |
| 257 | `GATT_FAILURE` (0x101) | Android's generic failure constant |

## Mapping precedence

Shell-synthesized checks run before any platform call, so platform errors
only surface for genuinely failing platform operations. Per GATT operation
the order is:

1. Adapter/backend initialization check (`Error::InvalidState` when the JNI
   bridge was not initialized, or `initialize()` was not called).
2. Connection check (`Error::NotConnected` when the link is down; the
   registry entry carries no fork handle on `connect` -> `Error::NotFound`).
3. Property-bit check (`Error::Att` code `0x03` when the write mode does not
   match the characteristic properties) / subscribe property check
   (`Error::InvalidState` when the characteristic has neither `NOTIFY` nor
   `INDICATE`).
4. Session-identifier resolution against the discovered tree (`Error::Att`
   invalid handle when it does not resolve).
5. Payload-length check (`Error::InvalidLength { len, max }` with `max`
   exactly the `max_write_len` value) - mandatory on Android, which
   silently truncates oversize payloads at the stack level.
6. The platform call, whose failures go through the tables below.

## Structural mapping (btleplug variant -> rblue error)

| btleplug error | rblue error | Notes |
|---|---|---|
| `NotConnected` | `Error::NotConnected` | The fork's `NotConnectedException`: a connect attempt exhausted (including its internal 133/0x3E retries), an operation on a disconnected device, or `connectGatt` returning null |
| `PermissionDenied` | `Error::Permission(BluetoothPermissionDenied)` | The fork raises this only for the `SecurityException` around `connectGatt` - `BLUETOOTH_CONNECT` on API 31+ |
| `NoAdapterAvailable` | `Error::Permission(BluetoothOff)` | `NoBluetoothAdapterException`: `getBluetoothLeScanner()` returns null on a powered-off radio; the no-BLE-hardware case is indistinguishable through the fork and shares the mapping (a switched-off radio is the common real-world cause) |
| `TimedOut(duration)` | `Error::Timeout` | Not authored by the droidplug fork today (its `connect_with_timeout` wrapper); normalized for future-proofing |
| `RuntimeError(message)` | see the two tables below | The main path: fork-authored failure strings, including the embedded GATT statuses |
| `DeviceNotFound`, `UnexpectedCallback`, `UnexpectedCharacteristic`, `NoSuchCharacteristic`, `NotSupported`, `Uuid`, `InvalidBDAddr`, `Other` | `Error::Backend(message)` | Fallback for everything with neither a status number nor a permission shape (`InvalidBDAddr` would mean the fork mis-parsed a JNI address string; `Other` covers JNI machinery failures) |

## `RuntimeError` message mapping

Recognized in order; the first match wins.

### 1. Embedded GATT status (`"... status: N"`)

The number must sit at the end of the message (the fork's stable format, see
the transit table); a `status:` token followed by more prose stays
unstructured. On match: `Error::Platform { code: N, message: <full text> }`.

### 2. Permission-denial texts (`SecurityException.getMessage()`)

`BluetoothLeScanner.startScan` throws synchronously when the runtime
permission is missing; the message text is the only signal (there is no
numeric code). Matching is case-insensitive on the stable permission-name
substrings:

| message contains | rblue error |
|---|---|
| `ACCESS_FINE_LOCATION` / `ACCESS_COARSE_LOCATION` / `location permission` | `Error::Permission(LocationDenied)` - the API 30-and-below scan denial (`"Need ACCESS_COARSE_LOCATION or ACCESS_FINE_LOCATION permission to get scan results"`) |
| `BLUETOOTH_SCAN` / `BLUETOOTH_CONNECT` / `android.permission.BLUETOOTH` | `Error::Permission(BluetoothPermissionDenied)` - the API 31+ denial (`"Need android.permission.BLUETOOTH_SCAN permission for ..."`); `BLUETOOTH_CONNECT` denials surface through the structured `PermissionDenied` variant instead when they come from `connectGatt` |
| any other `permission` mention | `Error::Permission(BluetoothPermissionDenied)` - a permission-shaped text in the BLE surface is a Bluetooth/location denial; the generic reason is the conservative reading |

### 3. Anything else

`Error::Backend` with the message preserved verbatim - e.g.
`"Unable to reconnect to device"` (the fork's reconnection-attempt failure,
no status), `"Unable to discover services"` (the synchronous
`discoverServices()` rejection), and `"Java exception: ..."` wrappers around
unexpected throwables.

## Errors synthesized by the shell itself

| situation | rblue error |
|---|---|
| `Adapter::initialize` (or any operation fetching the platform adapter) before `rblue_android::init(&mut env)` ran | `Error::InvalidState("backend not initialized; call rblue_android::init() with the application JNIEnv first")` |
| trait method called before `initialize` | `Error::InvalidState("adapter not initialized; call initialize() first")` |
| `stop_scan` with no scan running | `Error::InvalidState("no scan is running")` |
| `peripheral(id)` with a `DeviceId` that is not `DeviceId::Mac`, or an address never recorded in this adapter instance's registry | `Error::NotFound(id)` carrying the queried id (Android's `connectGatt` could reach an arbitrary address, but the shell's registry is the contract source; scan first) |
| `request_device` | `Error::Unsupported { platform: "android", feature: "request_device" }` |
| `pair` (already bonded) | `Ok(())` without starting a new procedure |
| `pair` (SecurityException) | `Error::Permission(BluetoothPermissionDenied)` |
| `pair` (adapter off / missing) | `Error::Permission(BluetoothOff)` |
| `pair` (user rejected / ended in `BOND_NONE`) | `Error::Pairing(Rejected)` |
| `pair` (deadline while `BONDING`) | `Error::Pairing(TimedOut)` |
| `unpair` (not bonded) | `Ok(())` without calling `removeBond` |
| `unpair` (deadline while still bonded) | `Error::Pairing(TimedOut)` |
| `unpair` (`removeBond` missing on the device) | `Error::Unsupported { platform: "android", feature: "unpair" }` |
| `initialize` on a non-Android host build | `Error::Unsupported { platform: "android", feature: "adapter" }` (`default_adapter`) |
| `initialize` state resolution when the fork ever reports `PoweredOff`/`Unauthorized`/`Unavailable` | `Error::Permission(BluetoothOff)` / `Error::Permission(Unauthorized)` / `Error::Unsupported` - the fork reports a constant `Unknown` today; the arms keep the mapping total |
| `subscribe` on a characteristic with neither `NOTIFY` nor `INDICATE` | `Error::InvalidState("characteristic has neither notify nor indicate")` |
| write mode does not match the property bits | `Error::Att` code `0x03` (write not permitted) |
| write payload longer than `max_write_len` | `Error::InvalidLength { len, max }` - before the platform call (the stack would silently truncate) |
| stale/fabricated `CharacteristicId` | `Error::Att` code `0x01` (invalid handle) |

## Deliberate notes

- **Fork limitation: connect-failure statuses are swallowed.** The fork's
  Java layer retries fast 133/0x3E connect attempts internally (3 total) and
  then throws `NotConnectedException` - the raw number does not survive into
  Rust. Fixing this means editing fork internals (a status-carrying
  exception), which the thin-shell rule forbids; it is a candidate item for
  the fork's next upstream sync or the M4-evaluated Driver-track migration,
  where the Kotlin driver reports statuses directly. Until then,
  `Error::NotConnected` + the supervisor's generic backoff govern failed
  connects.
- **Fork limitation: disconnect events carry no reason.** The JNI
  disconnect notification passes only the address; the shell publishes
  `DisconnectReason::Unknown`. The supervisor's 133-specific
  disconnect-path special case cannot trigger from an Android event today.
- **Fork limitation: `onScanFailed` is dropped.** Scan-side failures
  (radio toggled off mid-scan, scanner registration failures) are silent;
  the scan simply stops yielding. A `BLUETOOTH_SCAN` denial at `start_scan`
  *is* reported (synchronous `SecurityException`).
- **`Error::Disconnected` is never produced by this backend.** A disconnect
  is delivered as a `DeviceDisconnected` *event*; the Android GATT surface
  does not report "the link dropped mid-operation" as a distinct error value
  (in-flight operations fail with their raw status, typically 19/22/133).
- **Idempotency is shell-provided or fork-provided.** `disconnect`
  pre-checks the connection state; `unsubscribe` relies on the fork's
  documented "unsubscribing a non-subscribed characteristic is not an
  error" behavior; `connect` pre-checks and returns success for a
  redundant connect.
