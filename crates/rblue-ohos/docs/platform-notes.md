# rblue-ohos platform notes (HarmonyOS / ArkTS driver)

Backend shape: the Rust side of the `BleDriver` protocol (plan section 3.6).
BLE on OHOS is only reachable from ArkTS (`@ohos.bluetooth.ble`); there is no
public C API a Rust crate could call, so an ArkTS driver implementation drives
the OS stack and is injected into [`crate::DriverRegistry`] through the
ohos-rs Node-API facade (feature `napi`). The protocol shape lives once in
`rblue-driver` and must not be forked per platform; the ArkTS interface and
`.d.ts` are generated from the Rust types.

Split of responsibilities:

- **This crate (Rust)**: request correlation by `req_id`, timeouts, event
  routing into `AdapterEvent` broadcasts, binary data-plane framing, the
  `rblue_api::Adapter`/`Peripheral` implementations, and the napi exports.
- **The ArkTS driver (M4, `bindings/arkts`)**: the actual `@ohos.bluetooth.*`
  calls, permission prompting, lifecycle hooks, and the Worker-thread plumbing
  described below.

## ArkTS driver essentials

The driver implements the `BleDriver` interface over these OHOS APIs
(API 10+ baseline, HarmonyOS NEXT):

| protocol operation | OHOS call |
|---|---|
| `connect` | `ble.createGattClientDevice(deviceId)` then `gattClient.connect()` - the device id is the MAC string of a device seen by a scan (OHOS only connects to observed devices; the registry's scan history is the identity source) |
| `discoverServices` | `gattClient.getServices()` - must run on every connection; OHOS-side caches can be stale after a firmware GATT change |
| characteristic read/write | `gattClient.readCharacteristicValue(...)` / `writeCharacteristicValue(...)` - write picks `WRITE` vs `WRITE_NO_RSP` per the `withResponse` flag; bulk bytes ride the binary frame, not JSON |
| value-change notifications | `gattClient.on('BLECharacteristicChange', ...)` after `setCharacteristicChangeNotification` (notify) or `setCharacteristicChangeIndication` (indicate) - **OHOS exposes distinct APIs, so the driver picks from the characteristic properties exactly as `Peripheral::subscribe` documents** |
| CCCD teardown | `getCharacteristic().forEach(c => gattClient.setCharacteristicChangeNotification(c, false))` (and the indication variant) on disconnect; the Rust core additionally gates delivery on its own subscription set, so values cannot leak across reconnects |
| `requestMtu` | `gattClient.setBLEMtuSize(mtu)` (API 10+; the counterpart of Android's `requestMtu`). Completes with the negotiated MTU, which the registry records into `Peripheral::mtu()` / `max_write_len()` |
| `readRssi` | `gattClient.getRssiValue()` - **pending real-device verification**; the capability table reports `RssiDuringConnection: false` and `Peripheral::rssi()` returns `Error::Unsupported` until it is verified on hardware. The protocol command exists and is exercised by the round-trip PoC |
| connection state | `gattClient.on('BLEConnectionStateChange', ...)` -> `DeviceConnected` / `DeviceDisconnected` events via `emitEvent` |

## Adapter state comes from `@ohos.bluetooth.access`

The adapter power/availability state is **not** in the `ble` module: the
driver reads `@ohos.bluetooth.access` (`access.getState()`,
`access.on('stateChange')`) and reports it as the `GetAdapterState` command
result and `StateChanged` events. Mapping: `STATE_OFF`/`STATE_TURNING_OFF`
-> `AdapterState::Off`/`TurningOff`, `STATE_ON` -> `PoweredOn`,
`STATE_BLE_ON`/`STATE_BLE_TURNING_ON` -> the corresponding BLE-only states,
`STATE_DISCONNECTED` (no stack) -> `Unavailable`. The `initialize` handshake
sources its state from the same module, so `Adapter::initialize` reflects the
user's Bluetooth toggle as `Error::Permission(BluetoothOff)` semantics (code
`2900003`, see `docs/error-mapping.md`).

## Permissions

Three-application (三方) HAR packaging has exactly one core Bluetooth
permission:

- **`ohos.permission.ACCESS_BLUETOOTH`** (`user_grant`) - required for all
  BLE central operations on API 10+. The HAR exposes a `requestPermission()`
  helper the application calls at a suitable moment; the library never pops
  the dialog by itself.
- **`ohos.permission.DISCOVER_BLUETOOTH` must NOT be declared by the
  three-application HAR by default.** It is a system-level / restricted
  system_basic permission that third parties cannot obtain; shipping it in
  `module.json5` fails HAP integration outright. Remove it unless the
  deployment is a system partner and the permission has been granted through
  the profile.
- Background scanning additionally needs location-class permissions
  (`ohos.permission.LOCATION_IN_BACKGROUND` and friends), mirroring the
  Android policy. This is why `Capability::BackgroundScan` is reported
  absent: the HAR does not request them by default, and the capability stays
  off until a deployment opts in and the runtime probe confirms it.
- Before every release, re-check the declared permission list against the
  current Huawei permission catalog - the restricted set grows, and a newly
  restricted entry fails integration the same way `DISCOVER_BLUETOOTH` does.

## Lifecycle (foreground/background)

The HAR exposes `onForeground` / `onBackground` hooks the application wires
into its ability lifecycle:

- **onBackground**: automatically `stopScan` (power and compliance; OHOS
  suspends scanning for background applications anyway). Active GATT
  connections are kept; the Rust core's scanning flag is reset through the
  normal `StopScan` completion path.
- **onForeground**: the hook may restart the last scan filter (the HAR keeps
  the last `StartScan` filter for this purpose). Event-stream subscribers do
  not need to resubscribe - the snapshot-then-live stream keeps flowing;
  devices seen while scanning was suspended simply were not reported.

## Dual ABI

Ship both `.so`s in the HAR:

- `arm64-v8a` (`aarch64-unknown-linux-ohos`) - HarmonyOS NEXT real devices.
- `x86_64` (`x86_64-unknown-linux-ohos`) - the emulator image.

Both need the OHOS NDK (`OHOS_NDK_HOME` for the CI job, plan section 7); the
`napi` feature builds this crate as the cdylib the HAR loads. Host machines
without the NDK can still `cargo check -p rblue-ohos --features napi`
(type-checking needs no linking) and run the full pure-Rust test suite.

## High-frequency notifications: run the driver on a Worker thread

`on('BLECharacteristicChange')` callbacks and the `ThreadsafeFunction` calls
back into Rust both run on the ArkTS main thread by default. For
high-rate notify streams (the hot path this backend exists for) the driver
should run in a Worker thread (`@ohos.worker`):

1. The main thread creates the Worker and hands it the driver object (or the
   worker creates it; only one side owns the `ble` connections).
2. The Worker consumes `BLECharacteristicChange`, serializes the binary
   frame, and pushes it through `emitFrame` - keeping `ArrayBuffer` copies
   and JSON parsing off the UI thread.
3. Requests flowing main-thread -> Rust -> TSFN -> driver must reach the
   Worker via its message port; the driver's `call` method is free to
   `postMessage` internally because the Rust side only requires that
   `completeRequest`/`emitEvent`/`emitFrame` are called exactly once per
   request/event, from any thread.

The napi bridge (ThreadsafeFunctions) is thread-safe by construction; the
constraint is purely on the ArkTS side (never block the UI JS thread - plan
section 9's JS-stall assertion in CI exists for this).

## Writes and MTU

- Unified write semantics are enforced in the shell before dispatch:
  connection check, property-bit check (mode mismatch = ATT `0x03`), then
  `Error::InvalidLength` for payloads over `max_write_len` - exactly
  `mtu - 3` for both write modes (ATT_MTU minus the 3-byte ATT header).
- The registry starts every session at the ATT default of 23 bytes
  (`max_write_len` 20). `setBLEMtuSize` success updates the session MTU;
  re-query `max_write_len` after reconnecting (MTU is per-connection).
- Write payloads travel on the binary data plane (`WritePayload` frame keyed
  by the same `reqId` as the JSON command), so the ArkTS driver reads them
  with plain `DataView` accessors - no base64, no JSON escaping.

## Testing

- The pure Rust core (request correlation, timeouts, frames, event routing,
  GATT semantics) is fully unit-tested on the host - `cargo test
  -p rblue-ohos` needs no hardware, no Node-API, no NDK.
- The napi facade is feature-gated (`--features napi`) and host-checkable;
  the real JS round trip is the M4 PoC in `docs/driver-roundtrip.md`, which
  also fronts the project's highest risk item (TSFN throughput on real
  hardware).
- Contract tests against real peripherals run on-device via HDC (`aa test`)
  per plan section 6; the ArkTS-side port is M4 scope.
