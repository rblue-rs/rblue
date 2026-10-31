# Driver round-trip PoC checklist (M4)

The napi two-way bridge is the single highest-risk item of the whole plan
(section 8): `ThreadsafeFunction` (TSFN) calls Rust -> ArkTS, `req_id`
correlation back ArkTS -> Rust, and real-device notification throughput. The
PoC was front-loaded to M1 in the plan; this checklist is what M4 executes on
real hardware before the ArkTS driver and HAR packaging are built out.

The Rust side is complete and host-tested (`cargo test -p rblue-ohos`:
req_id correlation, out-of-order completion, timeout cleanup, frame
encode/decode, write semantics). What remains unproven is only the JS-side
behavior over the real Node-API runtime.

## Scope and prerequisites

- Device: one HarmonyOS NEXT real device (`arm64-v8a`) + one peripheral
  (nRF52840 test peripheral from the hardware-in-loop kit, or any notify
  characteristic).
- Toolchain: OHOS NDK (`OHOS_NDK_HOME`), `rustup target add
  aarch64-unknown-linux-ohos`, ohos-rs build (the HAR's hvigor task or a
  standalone `cargo build --target aarch64-unknown-linux-ohos --features
  napi` cdylib).
- A minimal ArkTS harness app that:
  1. constructs the napi `Rblue` instance,
  2. provides a `BleDriver` object with the unified `call(call: DriverCall)`
     method,
  3. registers it with `rblue.registerDriver(driver)`,
  4. wires `completeRequest` / `emitEvent` / `emitFrame` callbacks back into
     the same instance.

## PoC steps and acceptance criteria

### 1. Handshake and echo round trip (control plane)

Steps:
1. Call `initialize()`; the driver answers `init` with a `Success` response
   for the request's `reqId` via `completeRequest`.
2. Issue a `getAdapterState` round trip and print the result.
3. Exercise out-of-order completion: issue 3 requests (e.g. three
   `getAdapterState` calls) and complete them in reverse order from ArkTS.

Accept:
- [ ] Every Rust future resolves with exactly the response carrying its own
      `reqId` (no cross-talk at 3 concurrent requests; the registry's
      host-test `requests_correlate_when_completed_out_of_order` already
      proves the Rust half).
- [ ] `reqId` values observed from ArkTS match the sequential `u64` sequence
      (1, 2, 3, ...); JSON round trip preserves them exactly.
- [ ] `initialize()` resolves only after the driver's completion (promise
      ordering holds).

### 2. Binary data-plane write/read round trip

Steps:
1. Connect to the test peripheral and discover services
   (`discoverServices`).
2. Write a 20-byte payload to a writable characteristic
   (`write(device, ch, data, true)`); the payload must leave Rust as a
   `WritePayload` frame (13-byte header + bytes), never base64/JSON.
3. Read the characteristic back and compare bytes.
4. Repeat with a 21-byte payload and confirm the Rust side rejects it with
   `InvalidLength { len: 21, max: 20 }` before any TSFN dispatch.

Accept:
- [ ] Written bytes == read bytes, header parsed with a plain ArkTS
      `DataView` (`reqId` little-endian u64, kind byte, `payloadLen` u32).
- [ ] The oversize write never reaches `driver.call` (log in the harness).
- [ ] `requestMtu(247)` negotiates and subsequent `maxWriteLen` grows to 244
      (proves `setBLEMtuSize` + registry MTU tracking on hardware).

### 3. Notification hot path and throughput (the risk item)

Steps:
1. Subscribe to the notify characteristic (`subscribe`), then register a
   notification sink (`setNotificationSink(device, cb)`).
2. Configure the peripheral to notify as fast as it can (nRF test firmware
   connection-event saturation; typical target 100-1000 notifications/s at
   20-244-byte values, bounded by the negotiated MTU and connection
   interval).
3. Record for 30 s: notifications/s delivered to the JS callback, end-to-end
   latency percentiles (Rust emit -> JS callback), and drops (the registry's
   bounded queues drop oldest; count them by sequence-numbering payloads).
4. Repeat with the ArkTS driver on a Worker thread (see
   `docs/platform-notes.md`) and compare.

Accept:
- [ ] Zero `reqId` misroutes and zero leaked routing entries across the run
      (registry warning "notification frame without a paired routing event"
      never fires for well-formed pairs).
- [ ] Sustained delivery >= the peripheral's emission rate with the drop
      counter stable (bounded queues absorbing consumer jitter, not
      systematically losing data); any sustained loss is explained by the
      drop policy, not by the bridge.
- [ ] The ArkTS main (UI) thread shows no jank while the Worker-thread
      driver saturates the link (harness frame-time trace; the plan's
      CI "JS 线程卡顿断言").
- [ ] Worker-thread variant sustains a materially higher rate than the
      main-thread variant, or the main-thread variant is proven adequate and
      the Worker requirement is dropped with measurements attached.

### 4. Failure and lifecycle paths

Steps:
1. Turn Bluetooth off mid-session; expect the driver to report
   `2900003`/state change and in-flight requests to fail with
   `Error::Platform` (never a string fallback).
2. Drop the peripheral link (power it off) with a write in flight; expect
   `Error::Disconnected` on the pending request and a `DeviceDisconnected`
   event.
3. Time out a request: park one driver completion in the harness for >
   `request_timeout` (15 s default; use a short override), confirm
   `Error::Timeout` and that a late completion is dropped silently.
4. Background the app during an active scan; confirm the HAR hook issued
   `stopScan` and foregrounding resumes it.

Accept:
- [ ] All four paths observed; no Rust panic, no JS exception, no leaked
      pending entry (the next request still completes normally afterwards).

## Exit criteria for the PoC

All checkboxes green on real hardware with numbers recorded in this file (or
linked). Gate: M4 continues building the ArkTS driver + HAR only after step 3
passes; a failure here reopens the transport design (e.g. per-method TSFNs,
`napi_deferred` promises instead of TSFNs) before more code is written on
top.

## Known host-side limits (documented, not blockers)

- `napi-ohos` 1.2 type-checks on the host (`cargo check -p rblue-ohos
  --features napi`) but cannot link without Node symbols - the `.so` build is
  inherently NDK/M4 work.
- `u64` request ids cross the JS boundary inside the JSON envelope; JS number
  precision (2^53) is irrelevant in practice because the registry allocates
  ids sequentially from 1. If a future need for arbitrary ids arises, the
  JSON field moves to a string form (protocol change, versioned).
- `cargo check --target aarch64-unknown-linux-ohos` may pass or fail on the
  host depending on toolchain availability; it is informational here - the
  authoritative target build happens under the OHOS NDK in M4.
