# rblue-corebluetooth platform notes (macOS / iOS)

Backend shape: a thin shell over the forked btleplug `corebluetooth`
implementation (`CBCentralManager` + delegate callbacks on a dedicated
Objective-C thread, bridged through channels). The shell maps the fork's
`Central`/`Peripheral` surface onto `rblue-api` and owns the device registry,
event snapshotting, and notification forwarding. Nothing here may be fixed by
editing fork internals; adapt in the shell.

## Capability matrix (plan section 2.3, macOS/iOS column)

| Capability | Value | Reason |
|---|---|---|
| ActiveScan | yes | CoreBluetooth scanning is active inquiry |
| GattClient | yes | Central role |
| RssiDuringConnection | yes | `CBPeripheral.readRSSI` on a connected peripheral |
| MtuNegotiation | no | macOS/iOS negotiate the ATT MTU automatically; no request API |
| MtuQuery | no | CoreBluetooth does not expose the negotiated MTU; use `max_write_len` |
| RequestDevicePicker | no | `request_device` returns `Error::Unsupported` |
| Pairing | no | Implicit pairing on encrypted-characteristic access; `pair()` returns `Error::Unsupported` |
| BackgroundScan | no (conservative) | Possible with the `bluetooth-central` background mode on iOS, but iOS then silently yields nothing without a service-UUID filter (see below). 1.0 reports `false` so callers do not rely on it |
| PassiveScan | no | CoreBluetooth has no passive scanning |
| GattServer / Advertise | no | Phase 2 |
| ScanResponse | no | iOS delivers consolidated advertisement data, not separate scan responses |
| ConnectionPriority | no | CoreBluetooth has no public connection-priority / interval request API; `set_connection_priority` returns `Error::Unsupported` |
| AdapterPowerControl | no | iOS cannot programmatically switch Bluetooth; macOS apps likewise have no supported CoreBluetooth API. `set_powered` returns `Error::Unsupported` |
| SystemConnectedPeripherals | yes | `retrieveConnectedPeripheralsWithServices` via the fork's `retrieve_peripherals`; returned handles still need a local `connect()` before GATT |

## System-connected peripherals

`Adapter::system_connected_peripherals` is implemented. The shell calls the
fork's `retrieve_peripherals` with the given service UUIDs (mapped to
`retrieveConnectedPeripheralsWithServices`) and inserts matches into the
registry without marking the rblue session connected. An empty service list
returns `Error::InvalidState`. The caller must still `connect()` locally
before GATT operations.

## Runtime requirements

- **tokio context mandatory.** The fork spawns its event tasks with
  `tokio::task::spawn` and uses `tokio::sync` primitives; every backend call
  (`initialize`, scan calls, GATT calls, `notifications()`) must run inside a
  tokio runtime.
- `initialize()` blocks until `centralManagerDidUpdateState`: adapter
  creation waits for the first state callback, so every later platform call
  is valid once `initialize` returns. Calling anything before `initialize`
  fails with `Error::InvalidState` instead of the platform's
  calls-before-state-callback being silently invalid.
- The authorization prompt (macOS: local network / Bluetooth TCC; iOS:
  `NSBluetoothAlwaysUsageDescription`) is triggered by adapter creation,
  i.e. during `initialize`. Denied authorization surfaces as
  `Error::Permission(Unauthorized)`; a powered-off radio as
  `Error::Permission(BluetoothOff)`.

## Device identity

- **`DeviceId::Uuid` is the only identity source.** CoreBluetooth hands out
  opaque `NSUUID` identifiers; there is no MAC, no `DeviceId::Mac`, and no
  address type. `peripheral()` lookups with any other id kind return
  `Error::NotFound` carrying the queried id.
- The UUID is stable per device **per app install**; reinstalling the app
  changes it. Persist `DeviceId::Uuid` together with the device name /
  service fingerprint (plan section 2.1) and reconnect by re-scanning when
  in doubt.
- The shell keeps every device ever sighted in its registry for the
  adapter's lifetime: `Error::NotFound` is reserved for devices that were
  never seen. Disconnects do not evict entries, so a supervisor can call
  `peripheral(id)` again after a drop and `connect()` to re-establish.

## Scanning

- **Hardware filtering supports service UUIDs only.**
  `scanForPeripheralsWithServices` receives `ScanFilter.services`; everything
  else (`name_prefix`, `rssi_threshold`, `manufacturer_data`) is applied in
  software on the full advertisement stream - mind the power cost.
- **iOS background scanning without a service-UUID filter receives
  nothing.** This is the first thing to check for "cannot find my device"
  reports on iOS: the app must (a) declare the `bluetooth-central` background
  mode and (b) pass at least one service UUID in the scan filter. Because of
  this coupling, `Capability::BackgroundScan` is conservatively `false`.
- The fork enables `CBCentralManagerScanOptionAllowDuplicatesKey`, so
  re-advertisements keep flowing as `DeviceUpdated` events while scanning.
- Discovered peripherals are snapshot-replayed on every `events()`
  subscription as fresh `DeviceDiscovered` events, then live events follow
  (snapshot-then-live; publication and replay are serialized under the
  registry lock, so no duplicates or losses at subscription time).
- A device already known to the shell that is re-sighted after a disconnect
  is published as `DeviceUpdated`, never as a second `DeviceDiscovered`.
- Advertisement payloads: iOS consolidates advertisement and scan response;
  `adv_flags` and `is_connectable` are always `None` (not exposed). When a
  device advertises several manufacturer-data records, the shell reports the
  one with the smallest company id to keep the single-entry model
  deterministic. `tx_power` is clamped to the `i8` model (out-of-range
  values are dropped).

## GATT behavior

- **Discovery is mandatory on every connection** (iOS requirement; also
  refreshes the shell's identifier cache). CoreBluetooth has no filtered
  discovery, so `discover_services(filter)` fetches the full tree and filters
  in software; identifiers for services outside the filter need an
  empty-filter rediscovery, exactly as the contract states.
- `CharacteristicId` instances are counted among same-UUID entries in the
  fork's `BTreeSet` order. Same-UUID duplicate services/characteristics are
  rare; if present, instance order follows UUID sort order rather than
  hardware discovery order (inherited limitation of the fork's `BTreeSet`).
- `DeviceServicesModified` (service change mid-connection) clears the
  identifier cache; there is no unified event for it, so a rediscovery is
  required before further GATT operations (existing ids resolve to
  `Error::Att` invalid-handle afterwards).
- **Reconnects need no rescan**: after a disconnect the fork drops the
  peripheral's internal state, so the shell re-retrieves the device via
  `retrievePeripheralsWithIdentifiers` (by identifier, no scan) on the next
  `connect()`. CoreBluetooth remembers identifiers across app restarts, which
  is what makes persisted `DeviceId::Uuid` reconnects work.
- Disconnect reason: CoreBluetooth reports no structured reason, so
  `DeviceDisconnected` events carry `DisconnectReason::Unknown`. The fork
  emits duplicate disconnect notifications; the shell publishes on state
  transitions only.
- CCCD semantics: subscriptions die with the connection (the fork's internal
  state, including CCCD state, is dropped on disconnect). After a reconnect,
  `subscribe` must be called again before notifications flow - matching the
  mock's contract behavior.

## Writes and MTU

- **Unified write semantics:** payloads longer than `max_write_len` are
  rejected with `Error::InvalidLength { len, max }` before any platform call.
  Native `writeValue(_:type:)` in the without-response mode **silently
  truncates** oversize data; rblue never does (plan section 2.4).
- `max_write_len` maps the `maximumWriteValueLength(for:)` semantics. The
  fork samples `maximumWriteValueLengthForType(.withoutResponse)` once per
  discovery and stores it as an ATT-MTU estimate (`value + 3`, or the default
  23 before discovery / right after connect). Both write modes therefore
  report `estimate - 3`:
  - `WithoutResponse`: exact (`maximumWriteValueLength(for: .withoutResponse)`).
  - `WithResponse`: assumed equal (`ATT Write Request` has the same 3-byte
    header as the command per the Core Spec PDU layouts). TODO: if the fork
    ever samples the `.withResponse` variant, use it per mode.
- Re-query `max_write_len` after every (re)connect + discovery instead of
  caching: the MTU is renegotiated per connection and can change mid-session.
- `mtu()` returns `Error::Unsupported` by contract - the real negotiated MTU
  is system-managed and not exposed; derive everything from `max_write_len`.

## App packaging (iOS / macOS)

- iOS `Info.plist` must contain `NSBluetoothAlwaysUsageDescription` (App
  Store submissions reject the build otherwise; the prompt is shown on first
  adapter use).
- Background BLE central work requires the `bluetooth-central` background
  mode in `Info.plist` (`UIBackgroundModes`). With state restoration
  (`CBCentralManagerOptionRestoreIdentifierKey`) planned for Phase 2; until
  then the backend does not opt in, so iOS relaunches do not restore central
  state.
- macOS 10.13+/iOS 11+ per the objc2-core-bluetooth API surface used by the
  fork.

## Testing

- Unit tests (32) cover the pure mapping/filtering/registry layers and run
  everywhere, including CI without Bluetooth hardware.
- Hardware/permission-dependent flows (real scan, connect, GATT) live in
  `tests/hardware.rs`, `#[ignore]`-gated; run explicitly on a real Mac with
  `cargo test -p rblue-corebluetooth -- --ignored`. Never run them in CI.
- The shared `rblue_api::contract` suite runs against the mock backend by
  default; a CoreBluetooth run requires a real echo peripheral and is also
  opt-in per the testing policy.
