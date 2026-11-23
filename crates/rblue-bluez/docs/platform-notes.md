# rblue-bluez platform notes (Linux / BlueZ)

Backend shape: a thin shell over the forked btleplug `bluez` implementation
(D-Bus to `bluetoothd` through bluez-async: `SetDiscoveryFilter` +
`StartDiscovery` for scanning, `Device1.Connect` for connecting,
`GattService1`/`GattCharacteristic1` for GATT). The shell maps the fork's
`Central`/`Peripheral` surface onto `rblue-api` and owns the device registry,
event snapshotting, and notification forwarding. Nothing here may be fixed by
editing fork internals; adapt in the shell. Phase 2 will replace the shell's
interior with `bluer` (see the migration section) - the public traits,
including the `backend_version()` probe, stay unchanged.

## Requirements

- A running `bluetoothd` on the system bus (`org.bluez`). If the daemon is
  absent, `initialize` fails with `Error::Backend` carrying
  `org.freedesktop.DBus.Error.ServiceUnknown` - a deployment failure, not an
  expected capability gap (see error-mapping.md).
- `libdbus` at runtime (bluez-async's transport, same requirement as inside
  the fork). The shell itself is pure Rust over D-Bus; no BlueZ headers are
  used.
- Any BlueZ from the last several years works; the characteristic `MTU`
  property used by `mtu()`/`max_write_len()` appeared around BlueZ 5.62
  (2021) - older stacks degrade to the conservative ATT default (below).

## Radio state

BlueZ reports a single `Powered` flag per adapter. The fork maps it to
`PoweredOn`/`PoweredOff`; the shell's `initialize` reports `PoweredOff` as
`Error::Permission(BluetoothOff)` - the user's Bluetooth toggle. A machine
without any adapter fails earlier, at adapter enumeration, with
`Error::Unsupported`. Live `StateChanged` events follow the `Powered`
property changes; the collapsed `AdapterState::Unknown` never surfaces from
this backend.

`Adapter::state` reads the same `Powered` flag (via the fork after
`initialize`, or a short bluez-async session before it). `set_powered`
writes the BlueZ `Powered` property and this backend advertises
`Capability::AdapterPowerControl`. If the D-Bus daemon refuses the change
(`NotAuthorized` / `AccessDenied` / `NotPermitted`), the call returns
`Error::Permission(Unauthorized)`.

## Capability matrix (plan section 2.3, Linux column)

| Capability | Value | Reason |
|---|---|---|
| ActiveScan | yes | BlueZ performs active LE scanning by default (scan requests for scan responses); the fork sets `Transport::Auto` |
| GattClient | yes | Central role |
| MtuQuery | yes | The D-Bus `MTU` property on `org.bluez.GattCharacteristic1` (BlueZ around 5.62+); see "Writes and MTU" for the fallback |
| MtuNegotiation | no | The stack negotiates the ATT MTU automatically; there is no client request API |
| RssiDuringConnection | no | BlueZ does not update the device `RSSI` property while connected - the value freezes at the last scan-time sighting. `rssi()` reports `Error::Unsupported` instead of presenting stale data |
| RequestDevicePicker | no | `request_device` returns `Error::Unsupported` |
| Pairing | no | BlueZ exposes `org.bluez.Device1.Pair` (and bluez-async wraps it as `pair_with_timeout`), but the fork's `Peripheral` surface does not expose it. Reported as absent rather than faked; `pair()` returns `Error::Unsupported`. Accessing an encrypted characteristic still triggers the system pairing agent |
| BackgroundScan | yes | Scanning is a daemon-side activity; a background process keeps receiving sightings as long as the D-Bus connection and the discovery request live (subject to the discovery refcount below and to power management such as `rfkill`/suspend) |
| PassiveScan | no | The fork always uses `Transport::Auto` |
| ScanResponse | no | BlueZ merges scan-response payloads into the cumulative device properties; payloads are not delivered separately |
| GattServer / Advertise | no | Phase 2 (this is exactly what the `bluer` migration unlocks) |
| WatchAdvertisements | no | Web-only concept |
| AdapterPowerControl | yes | BlueZ `org.bluez.Adapter1.Powered`, written through bluez-async `set_powered` |

## Device identity and the RPA rule

- **`DeviceId::Mac([u8; 6], AddressType)` is the only identity source.**
  BlueZ reports each device's MAC (`Address` property); the shell keys its
  registry on the raw address bytes (the `AddressType` annotation is
  metadata, not identity). The fork's own `PeripheralId` wraps the D-Bus
  object path, so the shell resolves the MAC through the device properties
  and matches events back to entries by object path. Any non-`Mac` id kind
  returns `Error::NotFound` carrying the queried id.
- **Address type:** BlueZ reports only `public` or `random` (`AddressType`
  property). The shell refines random addresses from the two most
  significant bits of the address (BLE Core Spec Vol 6 Part B 1.3): top bits
  `01` -> `ResolvablePrivate`, `11` -> `RandomStatic`, `00`
  (non-resolvable private) and `10` (reserved) -> `Unknown`. A sighting
  without a reported type stays `Unknown`.
- **RPAs rotate; BlueZ tolerates it.** BlueZ identifies a rotating device by
  its current address on every sighting, so each rotation shows up as (at
  first) a fresh registry entry. Reconnect across a rotation by re-scanning
  and matching on name/advertisement fingerprints (the core value-add
  layer's `reconnect_by_filter`); a stored MAC is worthless after rotation.
- **The registry keeps every device ever sighted for the adapter's
  lifetime.** Disconnects do not evict entries, so a supervisor can call
  `peripheral(id)` again after a drop and `connect()` to re-establish.
  `Error::NotFound` is reserved for devices this adapter instance never
  recorded.
- **BlueZ's device cache is imported at `initialize`.** bluetoothd persists
  discovered/paired devices across sessions (until `RemoveDevice`); the
  fork's event stream synthesizes a `DeviceDiscovered` (plus
  `DeviceConnected` for connected devices) for every cache entry when the
  shell subscribes. Those devices therefore appear in the registry and in
  the first `events()` snapshot with their *cached*, possibly stale
  advertisement data (name/RSSI as of the last sighting). BlueZ can connect
  to anything in this cache, which is why cache-known devices are
  connectable without a fresh scan.

## Scanning

- **`services` is pushed down to BlueZ's discovery filter**
  (`SetDiscoveryFilter`); non-matching devices are not reported by the daemon
  at all. `name_prefix`, `manufacturer_data`, and `rssi_threshold` are
  applied in software on the full advertisement stream - the full stream is
  still received over the air, so software filters cost the same radio power
  as no filter. If battery life matters, prefer a `services` filter (or a
  `manufacturer_data`-filterable device); see the plan's power note on
  name/RSSI threshold filtering.
- The fork sets `duplicate_data: true`, so bluetoothd re-emits property
  changes on every sighting instead of only on changes: RSSI updates flow
  continuously, and the shell publishes them as `DeviceUpdated` events.
  Expect a high event rate in dense environments; the event stream is a
  drop-oldest broadcast (64 events per subscription), and the core
  value-add layer's buffering is the place to tune.
- Property refreshes (RSSI, manufacturer data, service data, advertised
  services) are merged into the cached advertisement directly from the
  event payload - BlueZ's `PropertiesChanged` signals carry the complete new
  value, so no extra device-info round trips happen on the hot path. A
  device that was not yet registered when its property event arrives is
  recorded through a full sighting.
- Discovered peripherals are snapshot-replayed on every `events()`
  subscription as fresh `DeviceDiscovered` events, then live events follow
  (snapshot-then-live; publication and replay are serialized under the
  registry lock, so no duplicates or losses at subscription time).
- A device already known to the shell that is re-sighted after a disconnect
  is published as `DeviceUpdated`, never as a second `DeviceDiscovered`.
- **Discovery is refcounted across D-Bus clients.** BlueZ stops scanning
  only when the last client drops its request; `stop_scan()` removes this
  client's request and clears the shell's scanning flag, but the radio may
  keep scanning for other clients (GNOME/KDE settings, `bluetoothctl`). The
  shell's flag tracks only this adapter instance.
- Calling `start_scan` while a scan is running replaces the discovery filter
  (BlueZ applies `SetDiscoveryFilter` on a discovering adapter immediately).
- Advertisement payloads: BlueZ consolidates advertisement and scan response
  into cumulative device properties, so `adv_flags` and `is_connectable` are
  always `None` (no per-sighting advertisement type or flags over D-Bus).
  When a device advertises several manufacturer-data records, the shell
  reports the one with the smallest company id; service-data entries are
  sorted by UUID, keeping the single-entry model deterministic. `tx_power`
  is clamped to the `i8` model (out-of-range values are dropped).

## Connection lifecycle

- `connect()` checks the connection state first (a D-Bus property read),
  then runs `Device1.Connect` and waits for service discovery (the fork
  enforces a 30 s deadline - exceeding it yields `Error::Timeout`).
  Redundant connects succeed without touching the stack, and BlueZ's
  already-connected rejections are treated as success (see
  error-mapping.md).
- BlueZ connects to anything in its device cache; unlike Windows there is no
  observer-first restriction. An address bluetoothd has never seen cannot be
  connected through this backend (the shell's registry only knows devices it
  recorded - and `initialize` imported the BlueZ cache).
- Disconnects (local or remote) arrive as a `Connected: false` property
  change; the shell publishes `DeviceDisconnected` with
  `DisconnectReason::Unknown` - BlueZ's D-Bus API reports no structured
  reason - on state transitions only (repeated signals are collapsed).
- CCCD semantics: subscriptions die with the connection, and the
  session-scoped GATT tree is cleared on disconnect. After a reconnect,
  `discover_services` must run again and `subscribe` must be called again
  before notifications flow - matching the mock's contract behavior.
- The fork's peripheral objects are cheap handles over the session; a
  re-sighting may install a fresh object on the registry entry (tracked by a
  generation counter the notification forwarder follows). BlueZ object paths
  are stable, so in practice this only matters after an adapter reset hands
  out a new path for the same address.
- **A lost D-Bus connection kills the session.** If the D-Bus connection
  drops (daemon restart, user session teardown), the fork's event streams
  end and operations fail with D-Bus transport errors (`Error::Backend`);
  re-`initialize` on a fresh adapter instance. There is no reconnect-once
  semantic at this layer; the connection supervisor above is the place to
  retry.

## GATT behavior

- **Discovery is mandatory on every connection.** BlueZ has no filtered
  discovery, so `discover_services(filter)` fetches the full tree and
  filters in software; identifiers for services outside the filter need an
  empty-filter rediscovery, exactly as the contract states. The fork
  aggregates services, characteristics, and descriptors per UUID and
  collapses same-UUID entries (first wins), so instance indices are 0 in
  practice; same-UUID duplicates are not addressable on Linux (inherited
  fork limitation).
- BlueZ's own GATT attribute cache can serve stale trees for bonded devices
  after a firmware GATT-table change; see the deployment notes for the
  `/etc/bluetooth/main.conf` knob and the plan's forced-refresh note.
- Notifications flow only for characteristics subscribed through the CCCD
  (`StartNotify`/`StopNotify`); values are routed to the unified
  `CharacteristicId` via the current connection's tree and dropped when the
  tree is absent (before discovery / after disconnect). bluetoothd picks
  notification over indication when a characteristic supports both.
- A slow notification consumer first loses values inside the shell's 32-slot
  subscriber buffer (drop-oldest); the fork delivers each characteristic
  value as a D-Bus property change, so dbus buffering sits in front of it.
  The core value-add layer's buffering is the place to tune this.

## Writes and MTU

- **Unified write semantics:** payloads longer than `max_write_len` are
  rejected with `Error::InvalidLength { len, max }` before any platform
  call. Never truncated, never chunked.
- `max_write_len` is the negotiated ATT MTU minus the 3-byte ATT header, for
  both write modes - BlueZ negotiates one ATT MTU per connection and both
  write PDUs share the 3-byte header. Re-query after every (re)connect
  instead of caching: the MTU is renegotiated per connection.
- `mtu()` and `max_write_len()` read the fork's MTU, which comes from the
  D-Bus `MTU` property of the discovered characteristics. **Caveats:** the
  value requires a prior `discover_services` on the current connection, and
  on BlueZ releases without the property the fork falls back to the ATT
  default of 23 - so `max_write_len` conservatively reports 20 (`23 - 3`).
  Treat the value as an upper bound only after discovery on a recent stack.
- `MtuNegotiation` is absent (no request API); `mtu()` returns
  `Error::NotConnected` when the device is not connected.

## Pairing

- No active-pairing API through the fork (see the capability table). BlueZ
  pairs implicitly through the system pairing agent when an encrypted
  characteristic is accessed; pairing failures surface as ATT errors
  (`0x05` insufficient authentication, `0x08` insufficient authorization,
  `0x0F` insufficient encryption) from the characteristic operation, or as
  `org.bluez.Error.NotAuthorized` mappings (see error-mapping.md).

## Deployment notes

- **GATT development against changing firmware:** bluetoothd caches the GATT
  database of bonded devices (`[GATT]` section, `Cache` option in
  `/etc/bluetooth/main.conf`). When a peripheral's GATT table changes across
  firmware updates, stale caches surface as missing/ghost characteristics
  until the device is rebonded or the cache is disabled. For GATT-heavy
  development and CI, the bluer project's recommendation applies:
  ```ini
  [GATT]
  Cache = no
  Channels = 1
  ```
  (`Cache = no` avoids the stale attribute cache; `Channels = 1` restricts
  EATT to a single bearer, dodging peripheral firmwares with incomplete
  EATT support.)
- **Power tip for scans:** only `services` filters reach bluetoothd's
  discovery filter. `name_prefix`/`rssi_threshold`/`manufacturer_data`
  filtering happens in software on the full advertisement stream - the radio
  cost is identical to unfiltered scanning. For battery-sensitive paths,
  filter by service UUID.
- **CI containers** need `bluetoothd` running and a D-Bus system bus; the
  contract test suite runs against `rblue-mock` instead. The hardware tests
  in `tests/hardware.rs` are `#[ignore]`-gated for real Linux machines.

## Phase 2: bluer migration

The plan pins this backend as a shell so its interior can switch from
btleplug/bluez-async to `bluer` when GATT server / advertising support is
needed (both are out of btleplug's reach). The migration contract:

- `rblue_api::Adapter`/`Peripheral` and the facade layers stay unchanged;
  only `crates/rblue-bluez/src/{adapter,peripheral,mapping}.rs` interiors
  change.
- `backend_version()` is the runtime probe: it names the current build
  (`bluez/btleplug <rev>`); the bluer build reports a distinguishable
  prefix (`bluez/bluer ...`). Callers can detect the switch without a
  compile-time dependency change.
- The pure mapping/hub layers (error name table, address refinement,
  advertisement determinism, filters, tree conversion, broadcast hub) carry
  over as-is; `bluer` surfaces the same D-Bus error names, so the
  error-mapping contract survives the swap.

## Testing

- Unit tests (37+) cover the pure mapping/filtering/hub layers - D-Bus error
  name normalization including the ATT-code recovery from messages, address
  classification, advertisement building and property merging, scan
  filtering, tree conversion, notification routing, and the broadcast hub -
  and run everywhere, including CI without Bluetooth hardware or Linux.
- The adapter/peripheral glue is Linux-only (`cfg(target_os = "linux")`)
  because the fork's `platform` module (and bluez-async/libdbus) only builds
  there; verify with `cargo check -p rblue-bluez` on a Linux machine. The
  fork-type error round-trip tests are also Linux-only (they construct real
  `dbus::Error` values).
- **Only verifiable on a real Linux machine with Bluetooth hardware:**
  adapter enumeration and `Powered` transitions, discovery-filter pushdown
  behavior, `Device1.Connect` semantics including the 30 s service-discovery
  deadline, characteristic `MTU` property values, CCCD notification
  delivery, and the discovery refcount interaction with other D-Bus
  clients. Hardware flows should be `#[ignore]`-gated per the testing policy
  (`tests/hardware.rs`).
