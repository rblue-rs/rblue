# rblue-winrt platform notes (Windows / WinRT)

Backend shape: a thin shell over the forked btleplug `winrtble`
implementation (`BluetoothLEAdvertisementWatcher` for scanning,
`BluetoothLEDevice::FromBluetoothAddressAsync` + `GattSession` for
connecting, `GattDeviceService`/`GattCharacteristic` for GATT). The shell
maps the fork's `Central`/`Peripheral` surface onto `rblue-api` and owns the
device registry, event snapshotting, and notification forwarding. Nothing
here may be fixed by editing fork internals; adapt in the shell.

## Minimum OS

Windows 10 1809 (build 17763) or newer: `GattSession` - and with it
`MaxPduSize`/`max_write_len` - requires it. Older builds are not supported.

## Radio state

The fork maps `RadioState::On` -> `PoweredOn`, `RadioState::Off` ->
`PoweredOff`, and everything else (`Disabled`, `NotAvailable`, `Unknown`) ->
`CentralState::Unknown`. On `initialize`, `PoweredOff` and `Unknown` both
report `Error::Permission(BluetoothOff)` - the common real-world cause is
the user's Bluetooth toggle (Windows reports the settings toggle as
`Disabled`), and a machine without any radio fails earlier, at adapter
enumeration, with `Error::Unsupported`. Live `StateChanged` events keep the
honest `AdapterState::Unknown` for the collapsed states.

`Adapter::state` reads the same mapping after `initialize`. `set_powered`
returns `Error::Unsupported { feature: "set_powered" }`; this backend does
not advertise `Capability::AdapterPowerControl`.

## Capability matrix (plan section 2.3, Windows column)

| Capability | Value | Reason |
|---|---|---|
| ActiveScan | yes | The fork sets `BluetoothLEScanningMode::Active` on every scan |
| GattClient | yes | Central role |
| MtuQuery | yes | `GattSession.MaxPduSize`, tracked live by the fork's `MaxPduSizeChanged` handler; `mtu()` returns it while connected |
| MtuNegotiation | no | Windows negotiates the ATT MTU automatically; there is no client request API |
| RssiDuringConnection | no | No WinRT API; RSSI exists only on advertisements (scan time). The fork's `read_rssi` would return the last scan-time cache value, so `rssi()` reports `Error::Unsupported` instead of presenting stale data |
| RequestDevicePicker | no | `request_device` returns `Error::Unsupported` |
| Pairing | no | WinRT exposes `DeviceInformation.Pairing.PairAsync`, but the fork does not surface it. Reported as absent rather than faked; `pair()` returns `Error::Unsupported`. Accessing an encrypted characteristic still triggers the implicit system pairing dialog |
| BackgroundScan | yes | The watcher keeps running in the background subject to platform restrictions: a suspended desktop app needs a `BluetoothLEAdvertisementWatcherTrigger` background task to keep receiving. 1.0 does not register such triggers; foreground apps scan normally |
| PassiveScan | no | The fork always requests active scanning |
| ScanResponse | no | The fork consolidates advertisement + scan response into cumulative peripheral properties; payloads are not delivered separately |
| GattServer / Advertise | no | Phase 2 |
| WatchAdvertisements | no | Web-only concept |
| ConnectionPriority | yes | WinRT `RequestPreferredConnectionParameters` via the fork: `Low` → `PowerOptimized`, `Balanced` → `Balanced`, `High` → `ThroughputOptimized`. Requires an established local connection |
| AdapterPowerControl | no | WinRT `Radio.SetStateAsync` is not exposed by the fork and is not offered here; `set_powered` returns `Error::Unsupported` |
| SystemConnectedPeripherals | no | The forked WinRT backend already exposes `retrieve_peripherals`, but this backend keeps `Error::Unsupported` until a dedicated follow-up wires it through the shell |

## Device identity and the RPA rule

- **`DeviceId::Mac([u8; 6], AddressType)` is the only identity source.**
  The fork's `PeripheralId` wraps the 6-byte address directly. `peripheral()`
  lookups are keyed on the address bytes (the `AddressType` annotation is
  metadata, not identity); any other id kind returns `Error::NotFound`
  carrying the queried id.
- **Address type:** Windows reports only `Public` or `Random`
  (`BluetoothAddressType`). The shell refines random addresses from the two
  most significant bits of the address (BLE Core Spec Vol 6 Part B 1.3): top
  bits `01` -> `ResolvablePrivate`, `11` -> `RandomStatic`, `00`
  (non-resolvable private) and `10` (reserved) -> `Unknown`. A sighting
  without a reported type stays `Unknown`. The entry keeps the type from its
  first sighting so session identities stay stable.
- **RPA devices must be observed before they can be connected.** Windows can
  only `FromBluetoothAddressAsync` to addresses its watcher has seen in the
  current boot session; an RPA rotates about every 15 minutes, so a stored
  MAC is worthless after rotation (and after reboot). Reconnect across a
  rotation by re-scanning and matching on name/advertisement fingerprints
  (the core value-add layer's `reconnect_by_filter`). When the OS does not
  know the address, `connect()` fails with `Error::NotFound(id)` - the
  documented meaning of `NotFound` on this path.
- The shell keeps every device ever sighted in its registry for the
  adapter's lifetime: `Error::NotFound` is reserved for devices that were
  never seen (or whose address the OS never learned). Disconnects do not
  evict entries, so a supervisor can call `peripheral(id)` again after a
  drop and `connect()` to re-establish.

## Scanning

- **Nothing is pushed down to a hardware filter.** The fork deliberately
  keeps the OS advertisement filter empty - some Windows BLE drivers
  silently drop matching advertisements when a 128-bit service-UUID filter
  is set - and filters service UUIDs in the watcher handler. The shell
  applies `name_prefix`, `manufacturer_data`, and `rssi_threshold` in
  software as well. All filter fields are therefore software filters on
  Windows; mind the power cost.
- The watcher requests extended advertisements where the adapter supports
  them; Coded (long-range) PHY scanning is opt-in inside the fork and
  disabled by default (`Adapter::set_use_coded_phy` in the fork's API).
- Discovered peripherals are snapshot-replayed on every `events()`
  subscription as fresh `DeviceDiscovered` events, then live events follow
  (snapshot-then-live; publication and replay are serialized under the
  registry lock, so no duplicates or losses at subscription time).
- A device already known to the shell that is re-sighted after a disconnect
  is published as `DeviceUpdated`, never as a second `DeviceDiscovered`.
- Advertisement payloads: the fork consolidates advertisement and scan
  response into cumulative property state, so `adv_flags` and
  `is_connectable` are always `None` (the fork does not parse the flags AD
  structure and does not surface the advertisement type). When a device
  advertises several manufacturer-data records, the shell reports the one
  with the smallest company id to keep the single-entry model
  deterministic. `tx_power` is clamped to the `i8` model (out-of-range
  values are dropped).

## Connection lifecycle

- `connect()` runs `FromBluetoothAddressAsync`, creates the `GattSession`,
  and triggers the link with an uncached `GetGattServices` round trip. The
  shell checks the connection state first, so redundant connects are cheap
  and disconnected GATT calls fail with `Error::NotConnected` before
  reaching the platform.
- **`GattSession.MaintainConnection` is not set by the fork.** The
  connection lives as long as the shell holds the fork's peripheral object
  (which owns the `BluetoothLEDevice` and `GattSession`); dropping the
  adapter releases everything. TODO (fork-side): expose a
  `MaintainConnection` toggle so the stack re-establishes the link
  automatically after radio-level drops; until then, supervision belongs to
  the rblue core layer's connection supervisor.
- Reconnects after a disconnect need no re-scan when the OS still knows the
  address (it does for anything sighted this boot): the fork evicts the
  peripheral from its internal map on disconnect and a re-sighting creates a
  fresh object - the shell installs that fresh handle on the registry entry
  so subsequent connects and notifications use it. Connected entries keep
  their live handle.
- Disconnect reason: Windows reports no structured reason through the fork,
  so `DeviceDisconnected` events carry `DisconnectReason::Unknown`. The fork
  emits duplicate disconnect notifications (reply path + connection-status
  handler); the shell publishes on state transitions only.
- CCCD semantics: subscriptions die with the connection, and the
  session-scoped GATT tree is cleared on disconnect. After a reconnect,
  `discover_services` must run again and `subscribe` must be called again
  before notifications flow - matching the mock's contract behavior.
- The shell never calls `connect` on an address that was not observed:
  registry entries only exist for watcher-sighted devices, which is exactly
  the Windows constraint (see the RPA rule above).

## GATT behavior

- **Discovery is mandatory on every connection.** Windows has no filtered
  discovery, so `discover_services(filter)` fetches the full tree and
  filters in software; identifiers for services outside the filter need an
  empty-filter rediscovery, exactly as the contract states.
- The fork collapses same-UUID services and characteristics (its per-UUID
  maps keep only the first entry), so instance indices are always 0 in
  practice; same-UUID duplicates are not addressable on Windows (inherited
  fork limitation).
- Discovery failures may surface late: the fork returns `Ok` with an empty
  tree when the service fetch reports a non-success status, and the failure
  appears as an invalid-handle `Error::Att` on the first characteristic
  operation. See error-mapping.md.
- Notifications flow only for characteristics subscribed through the CCCD;
  values are routed to the unified `CharacteristicId` via the current
  connection's tree and dropped when the tree is absent (before discovery /
  after disconnect). The fork prefers indication over notification when a
  characteristic supports both.
- A slow notification consumer first loses values inside the fork's
  internal 16-slot broadcast buffer, then in the shell's 32-slot subscriber
  buffer (drop-oldest); the core value-add layer's buffering is the place
  to tune this.

## Writes and MTU

- **Unified write semantics:** payloads longer than `max_write_len` are
  rejected with `Error::InvalidLength { len, max }` before any platform
  call. Never truncated, never chunked.
- `max_write_len` is the negotiated ATT MTU (`GattSession.MaxPduSize`,
  tracked live by the fork's `MaxPduSizeChanged` handler) minus the 3-byte
  ATT header, for both write modes - Windows negotiates one ATT MTU per
  connection and both write PDUs share the 3-byte header. Re-query after
  every (re)connect instead of caching: the MTU is renegotiated per
  connection.
- `mtu()` returns the live negotiated value while connected (`MtuQuery`
  capability); `Error::NotConnected` otherwise.

## Pairing

- No active-pairing API through the fork (see the capability table).
  Windows pairs implicitly when an encrypted characteristic is accessed;
  pairing failures surface as ATT errors (`0x05` insufficient
  authentication, `0x0F` insufficient encryption) from the characteristic
  operation.

## Testing

- Unit tests (36) cover the pure mapping/filtering/hub layers - error
  normalization including the `HRESULT`/GATT-status extraction, address
  classification, advertisement building, scan filtering, tree conversion,
  notification routing, and the broadcast hub - and run everywhere,
  including CI without Bluetooth hardware or Windows.
- The adapter/peripheral glue is Windows-only (`cfg(target_os = "windows")`)
  and is verified by `cargo check -p rblue-winrt --target
  x86_64-pc-windows-msvc`; it cannot execute on other hosts.
- **Only verifiable on a real Windows machine with Bluetooth hardware:**
  radio enumeration and state transitions, watcher scan behavior against
  real advertisements (extended ads, scan-response consolidation),
  `FromBluetoothAddressAsync` connect semantics including the
  observer-first RPA rule, `GattSession.MaxPduSize` values, CCCD
  notification/indication delivery, and disconnect event deduplication.
  Hardware flows should be `#[ignore]`-gated per the testing policy.
