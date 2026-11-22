import 'dart:convert';
import 'dart:typed_data';

/// Dart models mirroring the serde JSON contracts of `rblue-api`.
///
/// The shapes are the ones `crates/rblue-c` produces (locked by its Rust
/// tests): snake_case fields, externally tagged enums, and the `Error` enum's
/// serde-default (PascalCase) variant tags. Every model parses the exact JSON
/// the C ABI hands over the event/response callbacks.

/// Why a permission-gated operation failed (`rblue_api::PermissionReason`).
enum PermissionReason {
  /// The system Bluetooth radio is switched off.
  bluetoothOff('bluetooth_off'),

  /// Location permission missing (Android API 30 and below scanning).
  locationDenied('location_denied'),

  /// A Bluetooth runtime permission was not granted.
  bluetoothPermissionDenied('bluetooth_permission_denied'),

  /// The platform denied access for policy reasons.
  unauthorized('unauthorized');

  const PermissionReason(this.wire);

  /// The serde form (snake_case variant name).
  final String wire;

  /// Parses the serde form.
  static PermissionReason fromJson(String name) =>
      PermissionReason.values.firstWhere(
        (reason) => reason.wire == name,
        orElse: () =>
            throw RblueContractError('unknown PermissionReason: $name'),
      );

  /// The serde form.
  String toJson() => wire;
}

/// Normalized pairing/bonding failure (`rblue_api::PairingStatus`).
class PairingStatus {
  /// The serde form; `rejected`, `timed_out`, ... or `{"Other": <code>}`.
  final Object json;

  /// Wraps an already-parsed serde value.
  const PairingStatus(this.json);

  /// Raw platform code when the status is `Other`, else null.
  int? get otherCode {
    final value = json;
    if (value is Map<String, Object?> && value['Other'] is int) {
      return value['Other'] as int;
    }
    return null;
  }
}

/// An ATT procedure error (`rblue_api::AttError`).
class AttError {
  /// ATT error code (0x01..=0x7F protocol-defined, higher profile-defined).
  final int code;

  /// Attribute handle, when the platform reports it.
  final int? handle;

  /// Parses the serde form.
  factory AttError.fromJson(Map<String, Object?> json) =>
      AttError(json['code']! as int, json['handle'] as int?);

  const AttError(this.code, this.handle);
}

/// Kinds of the unified error JSON (`rblue_api::Error` serde variants,
/// PascalCase tags as the serde derive emits them).
enum RblueErrorKind {
  unsupported('Unsupported'),
  permission('Permission'),
  notFound('NotFound'),
  notConnected('NotConnected'),
  timeout('Timeout'),
  pairing('Pairing'),
  att('Att'),
  platform('Platform'),
  invalidLength('InvalidLength'),
  disconnected('Disconnected'),
  invalidState('InvalidState'),
  backend('Backend'),
  invalidArgument('InvalidArgument');

  const RblueErrorKind(this.wire);

  /// The serde form (PascalCase variant tag of `rblue_api::Error`).
  final String wire;
}

/// A failed rblue request, parsed from the response's `status_json`.
///
/// `Platform` keeps the raw platform status code (Android GATT status 133 and
/// friends) in [platformCode]; retry policies match on it. The message is
/// human-readable context, never a substitute for the code.
class RblueException implements Exception {
  /// Which unified error variant this is.
  final RblueErrorKind kind;

  /// Raw platform status code (`Platform { code }`), when present.
  final int? platformCode;

  /// Human-readable context (message field or free-form payload).
  final String message;

  /// The offending device JSON (`NotFound`), when present.
  final Object? device;

  /// ATT error detail (`Att`), when present.
  final AttError? att;

  /// Permission reason (`Permission`), when present.
  final PermissionReason? permissionReason;

  /// The original `status_json` string, for diagnostics.
  final String rawJson;

  /// Parses a `status_json` string (`rblue_api::Error` serde form or the
  /// facade-only `InvalidArgument` extension).
  factory RblueException.fromJson(String statusJson) {
    final value = jsonDecode(statusJson);
    // Serde emits variants without fields as JSON strings.
    final (kindName, payload) = switch (value) {
      'NotConnected' || 'Timeout' || 'Disconnected' => (value as String, null),
      final Map<String, Object?> map when map.length == 1 => (
        map.keys.single,
        map.values.single,
      ),
      _ => throw RblueContractError('malformed status JSON: $statusJson'),
    };
    final kind = RblueErrorKind.values.firstWhere(
      (kind) => kind.wire == kindName,
      orElse: () => RblueErrorKind.backend,
    );
    String message = kindName;
    Object? device;
    AttError? att;
    PermissionReason? permissionReason;
    int? platformCode;
    switch (kind) {
      case RblueErrorKind.unsupported:
        final map = _map(payload);
        message = 'unsupported on ${map['platform']}: ${map['feature']}';
      case RblueErrorKind.permission:
        permissionReason = PermissionReason.fromJson(payload! as String);
        message = 'permission denied: $permissionReason';
      case RblueErrorKind.notFound:
        device = payload;
        message = 'device not found';
      case RblueErrorKind.pairing:
        message = 'pairing failed: $payload';
      case RblueErrorKind.att:
        att = AttError.fromJson(_map(payload));
        message = 'GATT procedure failed (code ${att.code})';
      case RblueErrorKind.platform:
        final map = _map(payload);
        platformCode = map['code']! as int;
        message = map['message']! as String;
      case RblueErrorKind.invalidLength:
        final map = _map(payload);
        message = 'data length ${map['len']} exceeds max ${map['max']}';
      case RblueErrorKind.invalidState:
      case RblueErrorKind.backend:
        message = payload! as String;
      case RblueErrorKind.invalidArgument:
        message = _map(payload)['message']! as String;
      case RblueErrorKind.notConnected:
      case RblueErrorKind.timeout:
      case RblueErrorKind.disconnected:
        message = kindName;
    }
    return RblueException._(
      kind: kind,
      platformCode: platformCode,
      message: message,
      device: device,
      att: att,
      permissionReason: permissionReason,
      rawJson: statusJson,
    );
  }

  /// A client-side failure (never a backend status): timed-out request or a
  /// synchronous rejection code of `rblue_request`.
  factory RblueException.client(String message) => RblueException._(
    kind: RblueErrorKind.backend,
    message: message,
    rawJson: '',
  );

  const RblueException._({
    required this.kind,
    required this.message,
    required this.rawJson,
    this.platformCode,
    this.device,
    this.att,
    this.permissionReason,
  });

  /// True for the Android GATT status 133 connect-failure special case.
  bool get isAndroidGattStatus133 =>
      kind == RblueErrorKind.platform && platformCode == 133;

  @override
  String toString() => 'RblueException(${kind.name}): $message';

  static Map<String, Object?> _map(Object? payload) =>
      payload! as Map<String, Object?>;
}

/// A value did not match the rblue-c JSON contract; always a bug on one side.
class RblueContractError implements Exception {
  /// What was malformed.
  final String message;

  /// Creates the error.
  RblueContractError(this.message);

  @override
  String toString() => 'RblueContractError: $message';
}

/// Address kind of a MAC identity (`rblue_api::AddressType`).
enum AddressType {
  /// IEEE-registered public address.
  public('public'),

  /// Stable for the device's power cycle lifetime.
  randomStatic('random_static'),

  /// Rotates about every 15 minutes; never a persistent key.
  resolvablePrivate('resolvable_private'),

  /// The platform did not report the kind.
  unknown('unknown');

  const AddressType(this.wire);

  /// The serde form (snake_case variant name).
  final String wire;

  /// Parses the serde form.
  static AddressType fromJson(String name) => AddressType.values.firstWhere(
    (type) => type.wire == name,
    orElse: () => throw RblueContractError('unknown AddressType: $name'),
  );

  /// The serde form.
  String toJson() => wire;
}

/// Platform-agnostic peripheral identity (`rblue_api::DeviceId`).
///
/// Externally tagged: `{"mac":[[..6 bytes..],"random_static"]}`,
/// `{"uuid":"..."}`, `{"web":"..."}`. Pass instances back to the client
/// verbatim via [json]; do not persist them (RPA rotation, reinstall UUIDs).
class DeviceId {
  /// The serde form as received from rblue-c.
  final Object json;

  const DeviceId._(this.json);

  /// Parses the serde form.
  factory DeviceId.fromJson(Object json) => DeviceId._(json);

  /// A MAC identity, for tests and fixtures.
  factory DeviceId.mac(List<int> bytes, AddressType type) => DeviceId._({
    'mac': [bytes, type.toJson()],
  });

  /// A platform UUID identity, for tests and fixtures.
  factory DeviceId.uuid(String uuid) => DeviceId._({'uuid': uuid});

  /// A web identity, for tests and fixtures.
  factory DeviceId.web(String id) => DeviceId._({'web': id});

  /// True for MAC identities.
  bool get isMac =>
      json is Map<String, Object?> && (json as Map)['mac'] != null;

  /// The six MAC bytes for MAC identities, else null.
  List<int>? get macBytes {
    final map = json;
    if (map is Map<String, Object?> && map['mac'] is List) {
      final tuple = map['mac']! as List<Object?>;
      return (tuple[0]! as List<Object?>).cast<int>();
    }
    return null;
  }

  /// The address type for MAC identities, else null.
  AddressType? get addressType {
    final map = json;
    if (map is Map<String, Object?> && map['mac'] is List) {
      final tuple = map['mac']! as List<Object?>;
      return AddressType.fromJson(tuple[1]! as String);
    }
    return null;
  }

  /// Short display form: MAC text, or the platform id.
  String get display {
    final bytes = macBytes;
    if (bytes != null) {
      return bytes
          .map((byte) => byte.toRadixString(16).padLeft(2, '0').toUpperCase())
          .join(':');
    }
    final map = json;
    if (map is Map<String, Object?>) {
      return (map['uuid'] ?? map['web'] ?? '?').toString();
    }
    return '?';
  }

  @override
  bool operator ==(Object other) =>
      other is DeviceId &&
      const DeepCollectionEquality().equals(other.json, json);

  @override
  int get hashCode => const DeepCollectionEquality().hash(json);

  @override
  String toString() => 'DeviceId($display)';
}

/// Structural equality for JSON-shaped values.
class DeepCollectionEquality {
  /// Const constructor for the shared instance.
  const DeepCollectionEquality();

  /// Compares two JSON-shaped values structurally.
  bool equals(Object? a, Object? b) {
    if (identical(a, b)) return true;
    if (a is List && b is List) {
      return a.length == b.length &&
          a.asMap().entries.every((entry) => equals(entry.value, b[entry.key]));
    }
    if (a is Map && b is Map) {
      return a.length == b.length &&
          a.entries.every(
            (entry) =>
                b.containsKey(entry.key) && equals(entry.value, b[entry.key]),
          );
    }
    return a == b;
  }

  /// A structural hash.
  int hash(Object? value) {
    if (value is List) {
      return Object.hashAll(value.map(hash));
    }
    if (value is Map) {
      return Object.hashAll(
        value.entries.map((entry) => Object.hash(entry.key, hash(entry.value))),
      );
    }
    return value.hashCode;
  }
}

/// Parsed advertisement of one sighting (`rblue_api::AdvertisementData`).
class AdvertisementData {
  /// Advertised local name, when included.
  final String? localName;

  /// Service UUIDs advertised.
  final List<String> services;

  /// Service UUID and advertised service data bytes.
  final List<(String, List<int>)> serviceData;

  /// Advertised transmit power, when supplied.
  final int? txPower;

  /// Advertisement flags, when supplied.
  final int? flags;

  /// Signal strength of this sighting in dBm.
  final int? rssi;

  /// Whether this advertisement is connectable, when reported.
  final bool? isConnectable;

  /// Manufacturer-specific data: company ID and bytes.
  final (int, List<int>)? manufacturerData;

  /// Parses the serde form.
  factory AdvertisementData.fromJson(Map<String, Object?> json) {
    final manufacturer = json['manufacturer_data'];
    return AdvertisementData(
      localName: json['local_name'] as String?,
      services: (json['services']! as List<Object?>).cast<String>(),
      serviceData: ((json['service_data'] as List<Object?>?) ?? []).map((item) {
        final pair = item! as List<Object?>;
        return (pair[0]! as String, (pair[1]! as List<Object?>).cast<int>());
      }).toList(),
      txPower: json['tx_power'] as int?,
      flags: json['adv_flags'] as int?,
      rssi: json['rssi'] as int?,
      isConnectable: json['is_connectable'] as bool?,
      manufacturerData: manufacturer is List && manufacturer.isNotEmpty
          ? (
              manufacturer[0]! as int,
              (manufacturer[1]! as List<Object?>).cast<int>(),
            )
          : null,
    );
  }

  const AdvertisementData({
    required this.localName,
    required this.services,
    required this.rssi,
    required this.isConnectable,
    required this.manufacturerData,
    this.serviceData = const [],
    this.txPower,
    this.flags,
  });
}

/// One device seen during a scan (`rblue_api::DiscoveredDevice`).
class DiscoveredDevice {
  /// Stable-per-session identifier.
  final DeviceId id;

  /// Latest advertisement seen for the device.
  final AdvertisementData advertisement;

  /// Parses the serde form.
  factory DiscoveredDevice.fromJson(Map<String, Object?> json) =>
      DiscoveredDevice(
        id: DeviceId.fromJson(json['id']!),
        advertisement: AdvertisementData.fromJson(
          (json['advertisement']! as Map).cast<String, Object?>(),
        ),
      );

  const DiscoveredDevice({required this.id, required this.advertisement});

  /// Best display name: advertised name, else the identifier.
  String get displayName => advertisement.localName ?? id.display;
}

/// A peripheral known to the adapter; advertisement data may not be available.
class PeripheralInfo {
  final DeviceId id;
  final String? name;

  const PeripheralInfo({required this.id, this.name});

  factory PeripheralInfo.fromJson(Map<String, Object?> json) => PeripheralInfo(
    id: DeviceId.fromJson(json['id']!),
    name: json['name'] as String?,
  );
}

/// Radio/permission state (`rblue_api::AdapterState`).
enum AdapterState {
  /// The Bluetooth radio is powered off.
  poweredOff('powered_off'),

  /// The radio is on and the adapter is usable.
  poweredOn('powered_on'),

  /// Bluetooth permission not granted.
  unauthorized('unauthorized'),

  /// No Bluetooth hardware or platform support.
  unavailable('unavailable'),

  /// Not known yet.
  unknown('unknown');

  const AdapterState(this.wire);

  /// The serde form (snake_case variant name).
  final String wire;

  /// Parses the serde form.
  static AdapterState fromJson(String name) => AdapterState.values.firstWhere(
    (state) => state.wire == name,
    orElse: () => throw RblueContractError('unknown AdapterState: $name'),
  );
}

/// Why a connection dropped (`rblue_api::DisconnectReason`).
class DisconnectReason {
  /// The serde form: `"local"`, `"remote"`, `"timed_out"`, `"unknown"`, or
  /// `{"platform": <code>}`.
  final Object json;

  /// Wraps an already-parsed serde value.
  const DisconnectReason(this.json);

  /// Raw platform code when the reason is `platform`, else null.
  int? get platformCode {
    final value = json;
    if (value is Map<String, Object?> && value['platform'] is int) {
      return value['platform'] as int;
    }
    return null;
  }

  /// Short display text.
  String get display {
    final value = json;
    if (value is String) return value;
    if (value is Map<String, Object?> && value['platform'] != null) {
      return 'platform ${value['platform']}';
    }
    return 'unknown';
  }
}

/// GATT characteristic property bits
/// (`rblue_api::CharacteristicProperties`).
///
/// rblue-c serializes bitflags as "|"-separated uppercase flag names, e.g.
/// `"READ | WRITE_WITHOUT_RESPONSE | NOTIFY"`.
class CharacteristicProperties {
  /// 0x0002: value may be read.
  static const read = 0x0002;

  /// 0x0004: value may be written without a response.
  static const writeWithoutResponse = 0x0004;

  /// 0x0008: value may be written with a response.
  static const write = 0x0008;

  /// 0x0010: value changes are notified.
  static const notify = 0x0010;

  /// 0x0020: value changes are indicated.
  static const indicate = 0x0020;

  /// Parses the "|"-separated flag-name string.
  static int fromJson(String flags) {
    var bits = 0;
    for (final name in flags.split('|')) {
      bits |= switch (name.trim()) {
        'BROADCAST' => 0x0001,
        'READ' => read,
        'WRITE_WITHOUT_RESPONSE' => writeWithoutResponse,
        'WRITE' => write,
        'NOTIFY' => notify,
        'INDICATE' => indicate,
        'AUTHENTICATED_SIGNED_WRITES' => 0x0040,
        'EXTENDED_PROPERTIES' => 0x0080,
        'RELIABLE_WRITE' => 0x0100,
        'WRITABLE_AUXILIARIES' => 0x0200,
        _ => throw RblueContractError('unknown characteristic property: $name'),
      };
    }
    return bits;
  }
}

/// Identity of a service within a connection
/// (`rblue_api::ServiceId`). Valid only for the current connection.
class ServiceId {
  /// Service UUID.
  final String uuid;

  /// Zero-based index among same-UUID services.
  final int instance;

  /// Parses the serde form.
  factory ServiceId.fromJson(Map<String, Object?> json) => ServiceId(
    uuid: json['uuid']! as String,
    instance: json['instance']! as int,
  );

  const ServiceId({required this.uuid, required this.instance});

  /// The serde form.
  Map<String, Object?> toJson() => {'uuid': uuid, 'instance': instance};

  @override
  bool operator ==(Object other) =>
      other is ServiceId && other.uuid == uuid && other.instance == instance;

  @override
  int get hashCode => Object.hash(uuid, instance);
}

/// Identity of a characteristic within one connected session
/// (`rblue_api::CharacteristicId`). Valid only for the current connection:
/// after a reconnect, re-run `discoverServices` and use fresh ids.
class CharacteristicId {
  /// The service containing the characteristic.
  final ServiceId service;

  /// Characteristic UUID.
  final String uuid;

  /// Zero-based index among same-UUID characteristics.
  final int instance;

  /// Parses the serde form.
  factory CharacteristicId.fromJson(Map<String, Object?> json) =>
      CharacteristicId(
        service: ServiceId.fromJson(
          (json['service']! as Map).cast<String, Object?>(),
        ),
        uuid: json['uuid']! as String,
        instance: json['instance']! as int,
      );

  const CharacteristicId({
    required this.service,
    required this.uuid,
    required this.instance,
  });

  /// The serde form (the `write`/`read`/`subscribe` parameter).
  Map<String, Object?> toJson() => {
    'service': service.toJson(),
    'uuid': uuid,
    'instance': instance,
  };

  @override
  bool operator ==(Object other) =>
      other is CharacteristicId &&
      other.service == service &&
      other.uuid == uuid &&
      other.instance == instance;

  @override
  int get hashCode => Object.hash(service, uuid, instance);
}

/// One GATT characteristic (`rblue_api::Characteristic`).
class Characteristic {
  /// Characteristic UUID.
  final String uuid;

  /// Property bits; see [CharacteristicProperties].
  final int properties;

  /// Descriptor UUIDs (enumerated only in 1.0).
  final List<String> descriptors;

  /// Parses the serde form.
  factory Characteristic.fromJson(Map<String, Object?> json) => Characteristic(
    uuid: json['uuid']! as String,
    properties: CharacteristicProperties.fromJson(
      json['properties']! as String,
    ),
    descriptors: (json['descriptors']! as List<Object?>).cast<String>(),
  );

  const Characteristic({
    required this.uuid,
    required this.properties,
    required this.descriptors,
  });

  /// True when the value may be read.
  bool get readable => properties & CharacteristicProperties.read != 0;

  /// True when an acknowledged write is permitted.
  bool get writable => properties & CharacteristicProperties.write != 0;

  /// True when a best-effort write is permitted.
  bool get writableWithoutResponse =>
      properties & CharacteristicProperties.writeWithoutResponse != 0;

  /// True when the characteristic can deliver value updates.
  bool get notifiable =>
      properties &
          (CharacteristicProperties.notify |
              CharacteristicProperties.indicate) !=
      0;
}

/// One GATT service (`rblue_api::Service`).
class Service {
  /// Service UUID.
  final String uuid;

  /// True for a primary service.
  final bool primary;

  /// Characteristics contained in this service.
  final List<Characteristic> characteristics;

  /// Parses the serde form.
  factory Service.fromJson(Map<String, Object?> json) => Service(
    uuid: json['uuid']! as String,
    primary: json['primary']! as bool,
    characteristics: (json['characteristics']! as List<Object?>)
        .map(
          (value) =>
              Characteristic.fromJson((value! as Map).cast<String, Object?>()),
        )
        .toList(),
  );

  const Service({
    required this.uuid,
    required this.primary,
    required this.characteristics,
  });
}

/// The discovered GATT tree of one connection (`rblue_api::ServiceSet`).
class ServiceSet {
  /// All services returned by discovery.
  final List<Service> services;

  /// Parses the serde form.
  factory ServiceSet.fromJson(Map<String, Object?> json) => ServiceSet(
    services: (json['services']! as List<Object?>)
        .map(
          (value) => Service.fromJson((value! as Map).cast<String, Object?>()),
        )
        .toList(),
  );

  const ServiceSet({required this.services});

  /// Flattened characteristic list in discovery order.
  Iterable<Characteristic> get allCharacteristics =>
      services.expand((service) => service.characteristics);

  /// All characteristic IDs paired with their metadata in discovery order.
  Iterable<(CharacteristicId, Characteristic)>
  get identifiedCharacteristics sync* {
    final serviceCounts = <String, int>{};
    for (final service in services) {
      final serviceInstance = serviceCounts.update(
        service.uuid,
        (n) => n + 1,
        ifAbsent: () => 0,
      );
      final characteristicCounts = <String, int>{};
      for (final characteristic in service.characteristics) {
        final instance = characteristicCounts.update(
          characteristic.uuid,
          (n) => n + 1,
          ifAbsent: () => 0,
        );
        yield (
          CharacteristicId(
            service: ServiceId(uuid: service.uuid, instance: serviceInstance),
            uuid: characteristic.uuid,
            instance: instance,
          ),
          characteristic,
        );
      }
    }
  }

  /// The session id of the first characteristic with [uuid], if discovered.
  CharacteristicId? find(String uuid) {
    for (final (id, _) in identifiedCharacteristics) {
      if (id.uuid == uuid) return id;
    }
    return null;
  }
}

/// Write mode (`rblue_api::WriteMode`).
enum WriteMode {
  /// Acknowledged, reliable (needs the WRITE property). The default.
  withResponse('with_response'),

  /// Best-effort (needs WRITE_WITHOUT_RESPONSE).
  withoutResponse('without_response');

  const WriteMode(this.wire);

  /// The serde form (snake_case variant name).
  final String wire;

  /// The serde form.
  String toJson() => wire;
}

/// One GATT notification value (`rblue_api::Notification`).
class DeviceNotification {
  /// The device that delivered the value.
  final DeviceId device;

  /// Which characteristic delivered the value.
  final CharacteristicId id;

  /// The value bytes.
  final Uint8List data;

  /// Parses one `{"type":"notification",...}` envelope.
  factory DeviceNotification.fromJson(
    Map<String, Object?> json,
  ) => DeviceNotification(
    device: DeviceId.fromJson(json['device']!),
    id: CharacteristicId.fromJson(
      ((json['notification']! as Map)['id']! as Map).cast<String, Object?>(),
    ),
    data: Uint8List.fromList(
      ((json['notification']! as Map)['data']! as List<Object?>).cast<int>(),
    ),
  );

  const DeviceNotification({
    required this.device,
    required this.id,
    required this.data,
  });

  /// Value bytes as UTF-8 text, for display.
  String get dataAsText => utf8.decode(data, allowMalformed: true);

  /// Value bytes as hex, for display.
  String get dataAsHex =>
      data.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join(' ');
}

/// One adapter-level event, decoded from a tagged
/// `{"type":"adapter_event","event":{...}}` envelope.
sealed class AdapterEvent {
  const AdapterEvent();

  /// Parses one `AdapterEvent` object (the envelope's `event` field).
  factory AdapterEvent.fromJson(Object json) {
    final map = json as Map<String, Object?>;
    final tag = map.keys.single;
    final payload = map[tag];
    switch (tag) {
      case 'state_changed':
        return AdapterStateChanged(AdapterState.fromJson(payload! as String));
      case 'device_discovered':
        return AdapterDeviceDiscovered(
          DiscoveredDevice.fromJson((payload! as Map).cast<String, Object?>()),
        );
      case 'device_updated':
        final tuple = payload! as List<Object?>;
        return AdapterDeviceUpdated(
          DeviceId.fromJson(tuple[0]!),
          tuple[1]! as int,
        );
      case 'device_connected':
        return AdapterDeviceConnected(DeviceId.fromJson(payload!));
      case 'device_disconnected':
        final tuple = payload! as List<Object?>;
        return AdapterDeviceDisconnected(
          DeviceId.fromJson(tuple[0]!),
          tuple[1] == null ? null : DisconnectReason(tuple[1]!),
        );
      default:
        throw RblueContractError('unknown AdapterEvent tag: $tag');
    }
  }
}

/// Adapter power/permission state changed.
class AdapterStateChanged extends AdapterEvent {
  /// The new state.
  final AdapterState state;

  /// Builds the event.
  const AdapterStateChanged(this.state);
}

/// A new device was seen.
class AdapterDeviceDiscovered extends AdapterEvent {
  /// The discovered device with its latest advertisement.
  final DiscoveredDevice device;

  /// Builds the event.
  const AdapterDeviceDiscovered(this.device);
}

/// A known device re-advertised.
class AdapterDeviceUpdated extends AdapterEvent {
  /// The device.
  final DeviceId id;

  /// The new RSSI in dBm.
  final int rssi;

  /// Builds the event.
  const AdapterDeviceUpdated(this.id, this.rssi);
}

/// A device connection was established.
class AdapterDeviceConnected extends AdapterEvent {
  /// The device.
  final DeviceId id;

  /// Builds the event.
  const AdapterDeviceConnected(this.id);
}

/// A device connection dropped.
class AdapterDeviceDisconnected extends AdapterEvent {
  /// The device.
  final DeviceId id;

  /// Why the connection dropped, when known.
  final DisconnectReason? reason;

  /// Builds the event.
  const AdapterDeviceDisconnected(this.id, this.reason);
}

/// Bit positions of `rblue_api::Capability`, packed in the `capabilities`
/// bitmask (stored as a `u32` on the Rust side).
abstract final class Capability {
  /// See `Capability::PassiveScan`.
  static const int passiveScan = 1 << 0;

  /// See `Capability::ActiveScan`.
  static const int activeScan = 1 << 1;

  /// See `Capability::ScanResponse`.
  static const int scanResponse = 1 << 2;

  /// See `Capability::GattClient`.
  static const int gattClient = 1 << 3;

  /// See `Capability::GattServer`.
  static const int gattServer = 1 << 4;

  /// See `Capability::Advertise`.
  static const int advertise = 1 << 5;

  /// See `Capability::RssiDuringConnection`.
  static const int rssiDuringConnection = 1 << 6;

  /// See `Capability::MtuNegotiation`.
  static const int mtuNegotiation = 1 << 7;

  /// See `Capability::MtuQuery`.
  static const int mtuQuery = 1 << 8;

  /// See `Capability::Pairing`.
  static const int pairing = 1 << 9;

  /// See `Capability::WatchAdvertisements`.
  static const int watchAdvertisements = 1 << 10;

  /// See `Capability::RequestDevicePicker`.
  static const int requestDevicePicker = 1 << 11;

  /// See `Capability::BackgroundScan`.
  static const int backgroundScan = 1 << 12;

  /// Platform/stack background connection (`ConnectOptions.auto_connect`).
  /// This is not [RblueClient.autoConnect], which is the reconnect supervisor.
  static const int autoConnect = 1 << 13;

  /// Returns whether [bits] includes [capability].
  static bool has(int bits, int capability) => (bits & capability) != 0;
}

/// Serde shape of `rblue_api::ReconnectPolicy`. Durations are milliseconds.
/// Omitted fields keep the supervisor defaults.
class ReconnectPolicy {
  /// Backoff before the first retry.
  final Duration? initialBackoff;

  /// Cap for the base backoff.
  final Duration? maxBackoff;

  /// Growth factor per consecutive failure.
  final double? multiplier;

  /// Relative jitter applied to the base delay (0..=1).
  final double? jitterFraction;

  /// Base backoff after an Android GATT status 133 failure.
  final Duration? gatt133Backoff;

  /// Consecutive failures that open the circuit breaker; `null` disables it
  /// only when explicitly set in [toJson] via [breakerThresholdSet].
  final int? breakerThreshold;

  /// Whether [breakerThreshold] should be serialized, including a null.
  final bool _includeBreakerThreshold;

  /// Consecutive failures after which the supervisor gives up; `null` retries
  /// forever.
  final int? maxAttempts;

  /// Whether a local disconnect schedules a reconnect.
  final bool? reconnectOnLocalDisconnect;

  /// Optional jitter seed for reproducible tests.
  final int? jitterSeed;

  /// Builds a policy; omitted fields are left off the wire so the native
  /// defaults apply.
  const ReconnectPolicy({
    this.initialBackoff,
    this.maxBackoff,
    this.multiplier,
    this.jitterFraction,
    this.gatt133Backoff,
    this.breakerThreshold,
    bool includeBreakerThreshold = false,
    this.maxAttempts,
    this.reconnectOnLocalDisconnect,
    this.jitterSeed,
  }) : _includeBreakerThreshold =
           includeBreakerThreshold || breakerThreshold != null;

  /// The `*_ms` serde object sent as `auto_connect`'s `policy` field.
  Map<String, Object?> toJson() {
    return {
      if (initialBackoff != null)
        'initial_backoff_ms': initialBackoff!.inMilliseconds,
      if (maxBackoff != null) 'max_backoff_ms': maxBackoff!.inMilliseconds,
      if (multiplier != null) 'multiplier': multiplier,
      if (jitterFraction != null) 'jitter_fraction': jitterFraction,
      if (gatt133Backoff != null)
        'gatt_133_backoff_ms': gatt133Backoff!.inMilliseconds,
      if (_includeBreakerThreshold) 'breaker_threshold': breakerThreshold,
      if (maxAttempts != null) 'max_attempts': maxAttempts,
      if (reconnectOnLocalDisconnect != null)
        'reconnect_on_local_disconnect': reconnectOnLocalDisconnect,
      if (jitterSeed != null) 'jitter_seed': jitterSeed,
    };
  }
}

/// One event from the application-level connection supervisor.
sealed class SupervisorEvent {
  /// Builds a supervisor event.
  const SupervisorEvent();

  /// Parses one `SupervisorEvent` object (the envelope's `event` field).
  factory SupervisorEvent.fromJson(Object json) {
    final map = json as Map<String, Object?>;
    final tag = map.keys.single;
    final payload = map[tag];
    switch (tag) {
      case 'connected':
        return SupervisorConnected(DeviceId.fromJson(payload!));
      case 'reconnect_scheduled':
        final object = (payload! as Map).cast<String, Object?>();
        return SupervisorReconnectScheduled(
          DeviceId.fromJson(object['id']!),
          Duration(milliseconds: object['delay']! as int),
          object['attempt']! as int,
        );
      case 'attempt_failed':
        final object = (payload! as Map).cast<String, Object?>();
        final next = object['next_retry'];
        return SupervisorAttemptFailed(
          DeviceId.fromJson(object['id']!),
          platform133: object['platform_133']! as bool,
          attempt: object['attempt']! as int,
          nextRetry: next == null ? null : Duration(milliseconds: next as int),
        );
      case 'breaker_open':
        final object = (payload! as Map).cast<String, Object?>();
        return SupervisorBreakerOpen(
          DeviceId.fromJson(object['id']!),
          object['consecutive_failures']! as int,
        );
      case 'gave_up':
        final object = (payload! as Map).cast<String, Object?>();
        return SupervisorGaveUp(
          DeviceId.fromJson(object['id']!),
          object['consecutive_failures']! as int,
        );
      default:
        throw RblueContractError('unknown SupervisorEvent tag: $tag');
    }
  }
}

/// A watched device connected (or re-connected).
class SupervisorConnected extends SupervisorEvent {
  /// The device.
  final DeviceId id;

  /// Builds the event.
  const SupervisorConnected(this.id);
}

/// A watched device dropped and a reconnect was scheduled.
class SupervisorReconnectScheduled extends SupervisorEvent {
  /// The device that dropped.
  final DeviceId id;

  /// Backoff before the next attempt.
  final Duration delay;

  /// Consecutive failure count at scheduling time.
  final int attempt;

  /// Builds the event.
  const SupervisorReconnectScheduled(this.id, this.delay, this.attempt);
}

/// A connection attempt failed.
class SupervisorAttemptFailed extends SupervisorEvent {
  /// The device whose attempt failed.
  final DeviceId id;

  /// Whether the failure was Android GATT status 133.
  final bool platform133;

  /// Consecutive failure count after this failure.
  final int attempt;

  /// When the next attempt is scheduled, if any.
  final Duration? nextRetry;

  /// Builds the event.
  const SupervisorAttemptFailed(
    this.id, {
    required this.platform133,
    required this.attempt,
    this.nextRetry,
  });
}

/// The circuit breaker opened for this device.
class SupervisorBreakerOpen extends SupervisorEvent {
  /// The device.
  final DeviceId id;

  /// Consecutive failures at trip time.
  final int consecutiveFailures;

  /// Builds the event.
  const SupervisorBreakerOpen(this.id, this.consecutiveFailures);
}

/// The supervisor gave up on this device.
class SupervisorGaveUp extends SupervisorEvent {
  /// The device.
  final DeviceId id;

  /// Consecutive failures at give-up time.
  final int consecutiveFailures;

  /// Builds the event.
  const SupervisorGaveUp(this.id, this.consecutiveFailures);
}
