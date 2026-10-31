# rblue-ohos error mapping

Every platform failure is mapped into the unified `rblue_api::Error` at the
backend boundary. On OHOS the platform surface is the ArkTS `BleDriver`
(see `docs/platform-notes.md`), so the mapping has two legs: the driver
reports failures as structured `DriverResponse::Failure { reqId, code,
message }` values, and the Rust core turns them into
`Error::Platform { code, message }` with the raw numeric code preserved
verbatim - never demoted into an `Error::Backend` string (the same contract
that keeps Android GATT status 133 machine-readable for supervisors).

The code path: `DriverResponse::into_result()` in `rblue-driver` performs the
`Failure -> Error::Platform` conversion; the shell layers in `src/` add the
semantic checks below in front of every driver call. Numeric codes only ever
travel in the `code` field, and messages keep the driver's verbatim context.

## Mapping precedence

Shell-synthesized checks run before any driver dispatch, so platform errors
only surface for genuinely failing driver operations. Per GATT operation the
order is:

1. Connection check (`Error::NotConnected` when the link is down; a device
   the registry never recorded fails earlier with `Error::NotFound` at the
   `Adapter::peripheral` call site).
2. Property-bit check (`Error::Att` code `0x03` when the write mode does not
   match the characteristic properties) / subscribe property check
   (`Error::InvalidState` when the characteristic has neither `NOTIFY` nor
   `INDICATE`).
3. Payload-length check (`Error::InvalidLength { len, max }` with `max`
   exactly the `max_write_len` value).
4. The driver call, whose failures go through the tables below.

## Structural mapping (driver protocol -> rblue error)

| driver value | rblue error | Notes |
|---|---|---|
| `DriverResponse::Success { value }` | the operation's result | `DriverValue` is unwrapped per command; a mismatched kind (e.g. `Data` where `Services` was expected) is `Error::Backend` - a protocol violation, not a platform failure |
| `DriverResponse::Failure { code, message }` | `Error::Platform { code, message }` | The raw OHOS error code, preserved verbatim; see the code table below for what supervisors should expect |
| no transport registered | `Error::InvalidState("no driver transport registered")` | `registerDriver` has not run yet |
| transport delivery failure (TSFN queue closed/rejected) | `Error::Backend` | The request never reached the driver, so no platform code exists |
| request not completed within `RegistryConfig::request_timeout` | `Error::Timeout` | The pending entry is removed; a late completion is logged and dropped |
| `DeviceDisconnected` event while requests are in flight | `Error::Disconnected` | In-flight requests targeting that device fail fast instead of waiting out the deadline |
| undecodable / wrong-direction data-plane frame | `Error::Backend` | `FrameHeaderError` text preserved; `WritePayload` frames are host-to-core only |

## OHOS error-code table (for `Error::Platform { code }` consumers)

The OpenHarmony connectivity-kit documentation defines the `29000xx` family
for the Bluetooth APIs the ArkTS driver calls. The backend does not collapse
these into semantic variants (the raw code is the contract), but supervisors
and UI layers can match on them:

| code | OpenHarmony name | meaning | recommended supervisor treatment |
|---|---|---|---|
| `2900001` | `SERVICE_STOPPED` | The Bluetooth service is stopped (stack unavailable) | Retry after `initialize`; treat as `Permission(BluetoothOff)`-class unavailability for UI |
| `2900002` | `DEVICE_NOT_FOUND` | The device is not found (e.g. connect to an address never seen) | Map to the `Error::NotFound` flow; re-scan first - OHOS only connects to observed devices |
| `2900003` | `BLUETOOTH_SWITCH_OFF` | The Bluetooth switch is off | `Error::Permission(BluetoothOff)` semantics: prompt the user, then re-`initialize` |
| `2900004` | `PERMISSION_DENIED` | `ohos.permission.ACCESS_BLUETOOTH` (or a location-class permission for background scan) not granted | `Error::Permission(BluetoothPermissionDenied)` semantics: surface the HAR's `requestPermission()` helper |
| `2900005` | `GATT_ERROR` | A GATT procedure failed at the stack level | Retry logic per supervisor policy; ATT-level causes are not recoverable from this code alone |
| any other value | - | Driver-reported platform failure outside the documented family | Keep matching on the raw code; the message carries the driver's context |

GATT-level peer errors (read/write not permitted, insufficient
authentication, ...) may surface either as `GATT_ERROR` or as a
stack-specific code depending on the OHOS release; when the driver can
normalize an ATT status from the stack it should send it in `code`
(`0x02`..=`0x7F`), which `Error::Platform` still carries losslessly.

## Errors synthesized by the shell itself

| situation | rblue error |
|---|---|
| `stop_scan` with no scan running | `Error::InvalidState("no scan is running")` |
| `peripheral(id)` with a device never recorded by this registry | `Error::NotFound(id)` carrying the queried id (scan history is the identity source; the registry never evicts entries) |
| `request_device` | `Error::Unsupported { platform: "ohos", feature: "request_device" }` - OHOS has no user-gesture device picker |
| `pair` | `Error::Unsupported { platform: "ohos", feature: "pairing" }` - Phase 2; pairing happens implicitly when an encrypted characteristic is accessed |
| `rssi` on a connected device | `Error::Unsupported { platform: "ohos", feature: "read_rssi" }` - `getRssiValue` is pending real-device verification (capability `RssiDuringConnection` is not reported); the raw `ReadRssi` driver command remains available for the round-trip PoC |
| GATT operation before `discover_services` populated the session tree | `Error::InvalidState` naming the missing service/characteristic instance |
| `subscribe` on a characteristic with neither `NOTIFY` nor `INDICATE` | `Error::InvalidState("characteristic ... supports neither notify nor indicate")` |
| write with a mode the properties do not allow | `Error::Att { code: 0x03 }` (write not permitted) |
| payload longer than `max_write_len` | `Error::InvalidLength { len, max }` - never truncated, never hiddenly chunked |

## Deliberate notes

- **Numeric codes stay numeric.** The `29000xx` family and any driver-reported
  status land in `Error::Platform { code }`, matching the plan's rule that
  raw platform codes must remain machine-readable (Android's status 133
  pattern; supervisors special-case codes, never strings).
- **`Error::Disconnected` is event-driven.** A disconnect that is not racing
  an operation arrives as a `DeviceDisconnected` *event* (the driver's
  `DeviceDisconnected` event carries an optional normalized
  `DisconnectReason`); operations already parked on the failing link are
  failed fast with `Error::Disconnected`.
- **Idempotency is shell-provided.** `disconnect` on an already-down link
  returns `Ok(())` without dispatching; the driver protocol also declares
  `unsubscribe` idempotent.
- **Timeouts are a registry concern.** The driver protocol has no deadline of
  its own; `RegistryConfig::request_timeout` bounds every request and cleans
  the pending table, so a hung ArkTS driver cannot leak parked requests.
