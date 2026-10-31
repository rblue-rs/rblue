# rblue-mock error mapping

The mock backend has no platform stack: every `Error` it returns is either
synthesized from the simulated state (table below) or **injected verbatim by
the test** via `MockPeripheral::set_failure`. It never maps a real platform
status code, and `Error::Platform` can only originate from an injection.
`Error::Unsupported`, `Error::Permission`, `Error::Timeout`, `Error::Pairing`,
and `Error::Disconnected` are never produced (the mock reports every
capability bit and its operations complete synchronously, so nothing can
disconnect mid-operation).

## Synthesized errors

| Situation | Error |
|---|---|
| GATT operation (`read`/`write`/`subscribe`/`unsubscribe`/`max_write_len`/`discover_services`/`rssi`/`mtu`) while disconnected | `Error::NotConnected` |
| Session id (`CharacteristicId`) that does not resolve against the discovered tree (stale or fabricated) | `Error::Att(AttError { code: 0x01, handle: None })` - invalid handle |
| `read` on a characteristic without the `READ` property | `Error::Att` code `0x02` (read not permitted) |
| `write` in a mode the property bits do not allow (`WithResponse` needs `WRITE`, `WithoutResponse` needs `WRITE_WITHOUT_RESPONSE`) | `Error::Att` code `0x03` (write not permitted) |
| `subscribe` on a characteristic with neither `NOTIFY` nor `INDICATE` | `Error::InvalidState` |
| Payload larger than the characteristic's configured `max_write_len` | `Error::InvalidLength { len, max }` - never truncated, never chunked |
| `peripheral(id)` for a device never sighted and not connected | `Error::NotFound(id)` |
| `request_device` with no visible device matching the options | `Error::NotFound` (synthetic `DeviceId::Web`) |
| `stop_scan` with no scan running | `Error::InvalidState` |
| `discover_services` filtered to services the tree lacks | not an error: `Ok` with the matching (possibly empty) subset |

Injection precedence per operation: disconnected check, injected failure,
property/permission synthesis, payload-length check.

## Injected errors

`set_failure(op, characteristic, Some(failure))` makes every matching
operation fail with exactly that failure until cleared (`failure = None`
clears one entry, `clear_failures()` clears all). `characteristic = None`
applies the failure to every characteristic of the peripheral; a specific
characteristic UUID overrides the blanket entry.

| Injection | Delivered as |
|---|---|
| `MockFailure::Att(AttError { code, handle })` | `Error::Att` with the code and handle preserved |
| `MockFailure::att(code)` | `Error::Att` with `handle: None` |
| `MockFailure::Platform { code, message }` | `Error::Platform { code, message }` verbatim - supervisors match on the raw code |
| `MockFailure::platform(code)` | `Error::Platform` with message `"injected mock failure"` |

## Codes worth injecting

- **`133`** - Android `BluetoothGatt` `GATT_ERROR`, the leading connect/GATT
  failure status on fragmented Android stacks. The connection supervisor
  special-cases it: after 133 an immediate retry usually fails again, so the
  policy is backoff plus rebuilding the GATT connection from scratch
  (reconnect-by-filter when the RPA rotated). Inject with
  `MockFailure::platform(133)` to drive that state machine.
- **`0x05`** (`insufficient authentication`) and **`0x0F`**
  (`insufficient encryption`) - pairing/bonding triggered by GATT access;
  useful for testing flows that must call `pair()` and retry.
- **`0x08`** (`insufficient authorization`) - policy-level GATT rejection.
- **`0x0D`** (`invalid attribute value length`) - server-side length
  rejection, distinct from the client-side `Error::InvalidLength` check.
- **`0x11`** (`insufficient resources`) - backpressure/queue-overflow
  simulations.
