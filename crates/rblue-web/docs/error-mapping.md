# Web backend error mapping (rblue-web)

Every Web Bluetooth failure is mapped into the unified `rblue_api::Error`
model at the backend boundary; `DOMException` objects never leak through the
API. The authoritative implementation lives in
`crates/rblue-web/src/error_map.rs` (host-tested); this table documents it
per the rblue-api contract.

## DOMException name -> Error

| DOMException | Typical trigger | rblue Error |
|---|---|---|
| `NotFoundError` | `requestDevice`: the chooser was dismissed or no device matches the filters | `Permission(Unauthorized)` |
| `NotFoundError` | GATT operation addressing a device the page cannot access | `NotFound(DeviceId)` |
| `SecurityError` | `requestDevice` outside a user gesture, insecure context, or first access to a service absent from `optionalServices` | `Permission(Unauthorized)` |
| `NetworkError` | `gatt.connect()` failed (device out of range/asleep), or any GATT operation on a device whose link dropped | `Disconnected` |
| `AbortError` | GATT procedure aborted because the link dropped mid-operation | `Disconnected` |
| `NotSupportedError` | operation the browser does not implement (e.g. `writeValueWithResponse` on older builds) | `Unsupported { platform: "web", feature: <operation> }` |
| `InvalidStateError` | connecting a never-selected device, duplicate `startNotifications`, ... | `InvalidState("<name>: <message>")` |
| `TypeError` / `TypeMismatchError` | malformed options object or non-`BufferSource` payload | `InvalidState("<name>: <message>")` |
| anything else | unmapped browser-specific failure | `Backend("<name>: <message>")` |

The `feature` strings used by GATT operations: `connect`,
`discover_services`, `read`, `write_with_response`, `write_without_response`,
`subscribe`, `unsubscribe`. `requestDevice` failures use feature
`request_device`; the platform-level gap uses feature `bluetooth`.

## Notes

- The web platform reports **no numeric status codes** (there is no analog of
  Android GATT status 133), so `Error::Platform` is never produced by this
  backend. The supervisor's raw-code special-casing has nothing to match on
  here; disconnect-driven reconnection is the only recovery path.
- `Error::Disconnected` additionally surfaces as
  `AdapterEvent::DeviceDisconnected` (reason `Local` after an explicit
  `disconnect()`, otherwise `Unknown`) via the `gattserverdisconnected`
  event.
- `Error::InvalidLength` is raised in Rust *before* any browser call; the
  browser never sees an oversize payload (see `docs/platform-notes.md`).
- A thrown value that is not a `DOMException` (e.g. a bare string) maps to
  `Backend("UnknownError: ...")`.
