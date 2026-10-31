# rblue-android platform notes (Android / JNI)

Backend shape: a thin shell over the forked btleplug Android implementation
(droidplug): Rust calls the `com.nonpolynomial.btleplug.android.impl` helper
classes over JNI (scan via `BluetoothLeScanner`, connect via
`BluetoothDevice.connectGatt`, GATT via `BluetoothGatt`), and the Java side
calls back through registered native methods. The shell maps the fork's
`Central`/`Peripheral` surface onto `rblue-api` and owns the device registry,
event snapshotting, and notification forwarding. Nothing here may be fixed by
editing fork internals; adapt in the shell. The mixed Java/Rust build is the
platform's known pain point; the plan evaluates a Driver-track migration
(Kotlin implementing the `rblue-driver` protocol over a UniFFI callback
interface) after M4 - the public traits, including the `backend_version()`
probe, stay unchanged either way.

## Requirements

- **Android API 24+ (Android 7.0)** as the declared floor, packaged with the
  AAR.
- **The Java helper classes must be present at runtime.** They are produced
  by the fork's `scripts/build-java.sh` (`com.nonpolynomial.btleplug.**` plus
  the `io.github.gedgygedgy.rust` future/stream/ops support classes) and
  packaged into the AAR by the build orchestration; the shell's
  `init()` fails with `Error::Backend` when the classes cannot be resolved
  (typically a stripped or missing AAR payload).
- **ProGuard/R8 keep rules ship with the AAR's `consumer-rules.pro`**, so
  applications get the native-method and reflection keep rules without
  manual configuration. If an app strips the helper classes anyway, the
  failure mode is the `Error::Backend` from `init()` above.
- **Initialization order is strict.** `rblue_android::init(&mut env)` must
  run once with the `JNIEnv` of a thread attached to the application JVM,
  *from a thread that can see the application `ClassLoader`* - the fork
  resolves its helper classes through the calling thread's class loader and
  registers the JNI native methods there. The AAR's
  `Rblue.initialize(context)` Kotlin wrapper guarantees this by calling in
  from the application context. **Never call the Rust side from a `static`
  initializer**: the Android ClassLoader is not ready there and attached
  threads would resolve the wrong class loader. After `init()`, construct
  `default_adapter()` and call `Adapter::initialize()` from a tokio runtime.
- The AAR packaging (jniLibs with the `.so` for each ABI, e.g.
  `arm64-v8a`/`x86_64`) is owned by the build orchestration, not by this
  crate.

## Radio state

The fork's Android adapter tracks no radio state: `adapter_state()` is a
constant `CentralState::Unknown`, and no state events are emitted. The
shell's `initialize()` therefore reports `AdapterState::Unknown` and
**succeeds even on a powered-off radio** - the state is genuinely
unresolvable through the fork. The first real operation reports the radio
condition instead:

- `start_scan` with the radio off: `Error::Permission(BluetoothOff)`
  (`getBluetoothLeScanner()` returns null), or a permission denial
  (`Error::Permission`) if the runtime permission is missing.
- `connect` with the radio off: the connect attempt fails and surfaces as
  `Error::NotConnected` (the fork swallows the attempt's status; see
  error-mapping.md).

Live `StateChanged` events do not occur on this backend.

## Capability matrix (plan section 2.3, Android column)

| Capability | Value | Reason |
|---|---|---|
| ActiveScan | yes | The fork scans with `CALLBACK_TYPE_ALL_MATCHES` and default (active) scan settings |
| GattClient | yes | Central role over `BluetoothGatt` |
| MtuQuery | yes | The fork caches the negotiated MTU (its automatic `requestMtu(517)` at connect updates the value); `mtu()`/`max_write_len()` read it |
| MtuNegotiation | no | The fork's `Peripheral` trait surface exposes no MTU request API - the fork requests 517 automatically at connect, and the Java-side `requestMtu` is not surfaced. **TODO**: expose it via a fork PR or the Driver-track migration; the unified trait would also need a request method |
| RssiDuringConnection | yes | `readRemoteRssi` is implemented over the connected link (`rssi()`) |
| RequestDevicePicker | no | `request_device` returns `Error::Unsupported` |
| Pairing | no | Android's `createBond` is not exposed by the fork's `Peripheral` surface. **TODO**: fork PR or Driver-track migration. Accessing an encrypted characteristic still triggers the system pairing flow, and bonds created that way are used transparently |
| BackgroundScan | no (conservative) | Since API 31, scanning from the background requires a foreground service (`FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_CONNECTED_DEVICE` for connected-device work). The shell reports the capability as absent rather than promising behavior it cannot verify; document the foreground-service route in the app instead |
| PassiveScan | no | The fork always scans actively |
| ScanResponse | no | Android merges advertisement and scan response into one `ScanRecord`; payloads are not delivered separately |
| GattServer / Advertise | no | Phase 2 |
| WatchAdvertisements | no | Web-only concept |

## Permissions matrix (write these into the app manifest)

The shell maps *failures* (see error-mapping.md) but the declarations are the
application's responsibility:

| API level | scanning | connecting / GATT | background |
|---|---|---|---|
| 31+ (Android 12+) | `BLUETOOTH_SCAN` (runtime; add `neverForLocation` when the app does not derive location from scans) | `BLUETOOTH_CONNECT` (runtime; required for `connectGatt` and every GATT operation) | foreground service for background work: `FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_CONNECTED_DEVICE` (API 34+ enforcement) |
| 30 and below | `ACCESS_FINE_LOCATION` (runtime; `ACCESS_COARSE_LOCATION` suffices for some flows) | no Bluetooth permission (legacy static `BLUETOOTH`/`BLUETOOTH_ADMIN` are install-time) | location services must be enabled for scan results |

Denials surface as `Error::Permission` with the narrowest
`PermissionReason` the failure text identifies (`BluetoothPermissionDenied`
for the API 31+ runtime permissions, `LocationDenied` for the location
permission, `BluetoothOff` for a powered-off radio).

## Device identity and the RPA rule

- **`DeviceId::Mac([u8; 6], AddressType)` is the only identity source**;
  the fork's `PeripheralId` wraps a `BDAddr`. Android reports the MAC
  (`BluetoothDevice.getAddress()`), so no registry path mapping is needed -
  events are matched by raw address bytes.
- **The address type is always `AddressType::Unknown`.** The fork's
  scan-result parsing hard-codes `address_type: None`, and Android does not
  expose the address kind through `BluetoothDevice.getAddress()`. Unlike the
  BlueZ shell, no header-bit refinement is attempted: Android's reported
  bytes make no guarantee that the top two bits are the BLE address header.
- **RPAs rotate; every rotation looks like a new device.** Modern BLE
  peripherals rotate resolvable private addresses roughly every 15 minutes,
  so a stored MAC is worthless after rotation. Reconnect across a rotation
  by re-scanning and matching on name/advertisement fingerprints (the core
  value-add layer's `reconnect_by_filter`; `connect()` refreshes the cached
  name from the system's GAP name to support this).
- **The registry keeps every device ever sighted for the adapter's
  lifetime.** Disconnects do not evict entries, so a supervisor can call
  `peripheral(id)` again after a drop and `connect()` to re-establish.
  `Error::NotFound` is reserved for devices this adapter instance never
  recorded. Android itself could `connectGatt` to an arbitrary address, but
  the shell's registry is the contract source: scan first.

## Scanning

- **`services` is pushed down to the platform** (`ScanFilter` parcel,
  `setServiceUuid` per UUID); non-matching devices are not reported at all.
  `name_prefix`, `manufacturer_data`, and `rssi_threshold` are applied in
  software on the full advertisement stream - the radio cost of software
  filtering is identical to unfiltered scanning, so prefer a `services`
  filter for battery-sensitive paths.
- **`onScanFailed` is dropped by the fork**: radio-off-mid-scan and
  scanner-registration failures are silent; the scan simply stops yielding.
  The synchronous permission denial at `start_scan` *is* reported.
- Every sighting publishes the full properties snapshot: the first sighting
  is `DeviceDiscovered`, every re-sighting a `DeviceUpdated` (with RSSI) -
  Android re-delivers all matches, so expect a high event rate in dense
  environments. The event stream is a drop-oldest broadcast (64 events per
  subscription); the core value-add layer's buffering is the place to tune.
- Advertisement payloads: Android consolidates advertisement and scan
  response into one `ScanRecord`, and neither advertisement flags nor
  per-sighting connectability are exposed, so `adv_flags` and
  `is_connectable` are always `None`. When a device advertises several
  manufacturer-data records, the shell reports the one with the smallest
  company id; service-data entries are sorted by UUID, keeping the
  single-entry model deterministic. `tx_power` is clamped to the `i8` model
  (out-of-range values are dropped). Names containing U+FFFD replacement
  characters are stripped by the fork's parsing.
- Discovered peripherals are snapshot-replayed on every `events()`
  subscription as fresh `DeviceDiscovered` events, then live events follow
  (snapshot-then-live; publication and replay are serialized under the
  registry lock, so no duplicates or losses at subscription time).
- Calling `start_scan` while a scan is running replaces the filter.

## Connection lifecycle

- `connect()` pre-checks the connection state (redundant connects succeed
  without touching the stack), then runs the fork's connect: an internal
  command queue serializes GATT operations per device, and the Java layer
  **retries the fast-failing GATT status 133 / HCI 0x3E connect attempts
  itself** (up to 3 total attempts, only when the attempt failed quickly -
  on Android <= 14 a ~30 s direct-connect timeout also reports as 133, and
  retrying that would turn a connect to an absent device into a ~90 s
  wait). Exhausted attempts surface as `Error::NotConnected`; the raw
  attempt status does not survive (see error-mapping.md).
- After the link comes up, the fork **automatically requests MTU 517** and
  refreshes its properties snapshot with the system-cached GAP device name;
  the shell mirrors the name into its cached advertisement. The `connect`
  result therefore also covers MTU-negotiation failures, which *do* carry
  their status (`Error::Platform`).
- Disconnects (local or remote) arrive as a disconnect notification without
  a status code; the shell publishes `DeviceDisconnected` with
  `DisconnectReason::Unknown` on state transitions only (repeated signals
  are collapsed).
- CCCD semantics: subscriptions die with the connection, and the
  session-scoped GATT tree and subscription set are cleared on disconnect.
  After a reconnect, `discover_services` must run again and `subscribe`
  must be called again before notifications flow - matching the mock's
  contract behavior. This is enforced even for **bonded devices**, whose
  stack-side CCCD persistence can re-arm notifications automatically after
  a reconnect: values only flow while the characteristic is both discovered
  and subscribed on the current connection.
- The fork's peripheral objects are stable: one object per address for the
  adapter's lifetime, so the notification hub does not need to follow
  handle swaps (unlike the BlueZ shell's generation counter).
- **A lost JVM attachment kills the session.** JNI failures surface as
  `Error::Backend`; there is no reconnect-once semantic at this layer - the
  connection supervisor above is the place to retry.

## GATT behavior

- **Discovery is mandatory on every connection.** Android has no filtered
  discovery, so `discover_services(filter)` fetches the full tree and
  filters in software; identifiers for services outside the filter need an
  empty-filter rediscovery, exactly as the contract states. The fork walks
  the platform's GATT cache on every discovery, so a firmware GATT-table
  change is picked up on reconnection - with one caveat below.
- **`BluetoothGatt.refresh()` is not available.** The hidden method that
  flushes the stack's internal GATT cache (needed after a firmware upgrade
  on some vendor stacks) is not exposed by the fork, and reflection into it
  is a fork-internal change the thin-shell rule forbids. **TODO**: fork PR
  or Driver-track migration; until then, a stale-cache device must be
  forgotten/re-paired at the platform level or the cache dodged by
  reconnecting.
- The fork's Java layer deduplicates same-UUID characteristics per service
  while walking the cache and the Rust side keys services by UUID, so
  same-UUID duplicates are not addressable and instance indices are 0 in
  practice (inherited fork limitation, same as the BlueZ shell).
- `subscribe` writes the CCCD explicitly (Android requires it): the value
  is indication when the characteristic supports `INDICATE`, notification
  otherwise. A failed CCCD write reports its raw GATT status
  (`Error::Platform`). Values flow only for characteristics subscribed
  through the shell (`subscribe`), are routed to the unified
  `CharacteristicId` via the current connection's tree, and are dropped
  when the tree is absent (before discovery / after disconnect).
- A slow notification consumer first loses values inside the shell's
  32-slot subscriber buffer (drop-oldest); the core value-add layer's
  buffering is the place to tune this.

## Writes and MTU

- **Unified write semantics:** payloads longer than `max_write_len` are
  rejected with `Error::InvalidLength { len, max }` before any platform
  call. This is not optional on Android: the stack silently truncates
  oversize payloads (both write types enforce MTU - 3 per call).
- `max_write_len` is the negotiated ATT MTU minus the 3-byte ATT header,
  for both write modes. The fork requests MTU 517 at every connect; before
  that completes the value is the ATT default 23, so the conservative limit
  is 20. Re-query after every (re)connect instead of caching: the MTU is
  renegotiated per connection.
- `mtu()` and `max_write_len()` read the fork's cached MTU, which the
  automatic `requestMtu(517)` updates at connect; the platform may
  renegotiate later, and the cached value follows. `MtuNegotiation` (an
  explicit request API) is absent through the fork - see the capability
  table TODO.

## Pairing

- No active-pairing API through the fork (see the capability table). Android
  pairs implicitly when an encrypted characteristic is accessed and the
  platform decides bonding is required; pairing failures surface as ATT
  errors from the characteristic operation (typically 5 insufficient
  authentication / 8 insufficient authorization as raw GATT statuses in
  `Error::Platform`).

## Testing

- Unit tests (29+) cover the pure mapping/filtering/hub layers - GATT
  status recovery including the 133 rule per call site, permission-text
  classification, address handling, advertisement building, scan filtering,
  tree conversion, notification routing with the subscription gate, and the
  broadcast hub - and run everywhere, including CI without Android tooling.
- The adapter/peripheral glue is Android-only
  (`cfg(target_os = "android")`) because the fork's `platform` module and
  the JNI machinery only build there; verify with
  `cargo check -p rblue-android --target aarch64-linux-android` (and
  `x86_64-linux-android`). `check` does not link, so no NDK is required.
- **Only verifiable on real devices:** JNI class resolution from the
  application ClassLoader, permission-denial message texts across vendor
  skins, the fork's 133 connect-retry behavior, MTU auto-negotiation
  values, CCCD delivery, and the bonded-device CCCD persistence
  interaction. Hardware flows should be `#[ignore]`-gated per the testing
  policy; the contract test suite runs against `rblue-mock` in CI.
