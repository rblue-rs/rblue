import Foundation

// MARK: - JSON contract
//
// The UniFFI facade carries structured payloads as serde JSON strings using
// the snake_case, externally-tagged representations of the `rblue-api` types
// (see crates/rblue-ffi/src/lib.rs, "Serialization contract"). These types
// mirror those representations; event payloads received through the
// EventSink are decoded here, and identifiers are re-encoded verbatim when
// passed back into facade methods.

/// Radio/permission state of the adapter (`rblue_api::AdapterState`).
enum AdapterState: String, Decodable, Sendable {
    case poweredOff = "powered_off"
    case poweredOn = "powered_on"
    case unauthorized
    case unavailable
    case unknown
}

/// Why a connection dropped (`rblue_api::DisconnectReason`). CoreBluetooth
/// reports no structured reason, so the demo mostly sees `.unknown`.
enum DisconnectReason: Equatable, Sendable {
    case local
    case remote
    case timedOut
    case platform(Int32)
    case unknown
}

extension DisconnectReason: Decodable {
    private enum CodingKeys: String, CodingKey {
        case platform
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let raw = try? container.decode(String.self) {
            switch raw {
            case "local": self = .local
            case "remote": self = .remote
            case "timed_out": self = .timedOut
            case "unknown": self = .unknown
            case let other:
                throw DecodingError.dataCorrupted(.init(
                    codingPath: decoder.codingPath,
                    debugDescription: "unknown DisconnectReason \(other)"
                ))
            }
            return
        }
        let fields = try container.decode(Self.Fields.self)
        self = .platform(fields.platform)
    }

    /// Object form of the `Platform(i32)` tuple variant: `{"platform": N}`.
    private struct Fields: Decodable {
        let platform: Int32
    }
}

/// Address kind carried by a `DeviceId.mac` (`rblue_api::AddressType`).
enum AddressType: String, Codable, Sendable {
    case `public` = "public"
    case randomStatic = "random_static"
    case resolvablePrivate = "resolvable_private"
    case unknown
}

/// Platform-agnostic peripheral identifier (`rblue_api::DeviceId`).
///
/// Opaque by contract: never constructed from raw values, never persisted
/// (on iOS the UUID changes when the application is reinstalled). Decoded
/// values are re-encoded to canonical JSON when passed back into facade
/// methods, which the Rust side parses exactly like the original.
enum DeviceId: Hashable, Sendable {
    case mac([UInt8], AddressType)
    case uuid(String)
    case web(String)
}

extension DeviceId: Codable {
    private enum CodingKeys: String, CodingKey {
        case mac, uuid, web
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let uuid = try container.decodeIfPresent(String.self, forKey: .uuid) {
            self = .uuid(uuid)
            return
        }
        if let web = try container.decodeIfPresent(String.self, forKey: .web) {
            self = .web(web)
            return
        }
        if container.contains(.mac) {
            var fields = try container.nestedUnkeyedContainer(forKey: .mac)
            let bytes = try fields.decode([UInt8].self)
            let addressType = try fields.decode(AddressType.self)
            self = .mac(bytes, addressType)
            return
        }
        throw DecodingError.dataCorrupted(.init(
            codingPath: decoder.codingPath,
            debugDescription: "no known DeviceId variant key (mac/uuid/web)"
        ))
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .mac(let bytes, let addressType):
            var fields = container.nestedUnkeyedContainer(forKey: .mac)
            try fields.encode(bytes)
            try fields.encode(addressType)
        case .uuid(let value):
            try container.encode(value, forKey: .uuid)
        case .web(let value):
            try container.encode(value, forKey: .web)
        }
    }
}

extension DeviceId {
    /// Short human-readable form for list rows and headings.
    var summary: String {
        switch self {
        case .mac(let bytes, let addressType):
            let hex = bytes.map { String(format: "%02X", $0) }.joined(separator: ":")
            return "\(hex) (\(addressType.rawValue))"
        case .uuid(let value):
            return value
        case .web(let value):
            return value
        }
    }
}

/// One GATT notification payload (`rblue_api::Notification`).
struct Notification: Decodable, Equatable, Sendable {
    let id: CharacteristicId
    let data: [UInt8]
}

/// Parsed advertisement (`rblue_api::AdvertisementData`). iOS consolidates
/// advertisement and scan response; `advFlags`/`isConnectable` may be null.
struct AdvertisementData: Decodable, Equatable, Sendable {
    var localName: String?
    var services: [String]
    var serviceData: [ServiceDataEntry]
    var manufacturerData: ManufacturerData?
    var txPower: Int8?
    var advFlags: UInt8?
    var rssi: Int16?
    var isConnectable: Bool?

    /// Memberwise initializer for test fixtures; production values always
    /// come through `init(from:)`.
    init(
        localName: String?,
        services: [String],
        serviceData: [ServiceDataEntry],
        manufacturerData: ManufacturerData?,
        txPower: Int8?,
        advFlags: UInt8?,
        rssi: Int16?,
        isConnectable: Bool?
    ) {
        self.localName = localName
        self.services = services
        self.serviceData = serviceData
        self.manufacturerData = manufacturerData
        self.txPower = txPower
        self.advFlags = advFlags
        self.rssi = rssi
        self.isConnectable = isConnectable
    }

    private enum CodingKeys: String, CodingKey {
        case localName = "local_name"
        case services
        case serviceData = "service_data"
        case manufacturerData = "manufacturer_data"
        case txPower = "tx_power"
        case advFlags = "adv_flags"
        case rssi
        case isConnectable = "is_connectable"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        localName = try container.decodeIfPresent(String.self, forKey: .localName)
        services = try container.decodeIfPresent([String].self, forKey: .services) ?? []
        serviceData = try container.decodeIfPresent(
            [ServiceDataEntry].self, forKey: .serviceData) ?? []
        manufacturerData = try container.decodeIfPresent(
            ManufacturerData.self, forKey: .manufacturerData)
        txPower = try container.decodeIfPresent(Int8.self, forKey: .txPower)
        advFlags = try container.decodeIfPresent(UInt8.self, forKey: .advFlags)
        rssi = try container.decodeIfPresent(Int16.self, forKey: .rssi)
        isConnectable = try container.decodeIfPresent(Bool.self, forKey: .isConnectable)
    }
}

/// `(uuid, bytes)` service-data entry, serialized as a JSON pair.
struct ServiceDataEntry: Decodable, Equatable, Sendable {
    let uuid: String
    let data: [UInt8]

    init(from decoder: Decoder) throws {
        var fields = try decoder.unkeyedContainer()
        uuid = try fields.decode(String.self)
        data = try fields.decode([UInt8].self)
    }
}

/// `(company_id, bytes)` manufacturer-data entry, serialized as a JSON pair.
struct ManufacturerData: Decodable, Equatable, Sendable {
    let companyId: UInt16
    let data: [UInt8]

    init(from decoder: Decoder) throws {
        var fields = try decoder.unkeyedContainer()
        companyId = try fields.decode(UInt16.self)
        data = try fields.decode([UInt8].self)
    }
}

/// One device sighting (`rblue_api::DiscoveredDevice`).
struct DiscoveredDevice: Decodable, Equatable, Sendable {
    let id: DeviceId
    let advertisement: AdvertisementData
}

/// Streamed adapter events (`rblue_api::AdapterEvent`).
///
/// Externally tagged: `{"device_updated": [<DeviceId>, <rssi>]}` and similar;
/// single-field tuple variants serialize their payload directly.
enum AdapterEvent: Equatable, Sendable {
    case stateChanged(AdapterState)
    case deviceDiscovered(DiscoveredDevice)
    case deviceUpdated(DeviceId, rssi: Int16)
    case deviceConnected(DeviceId)
    case deviceDisconnected(DeviceId, DisconnectReason?)
}

extension AdapterEvent: Decodable {
    private enum CaseKey: String, CodingKey {
        case stateChanged = "state_changed"
        case deviceDiscovered = "device_discovered"
        case deviceUpdated = "device_updated"
        case deviceConnected = "device_connected"
        case deviceDisconnected = "device_disconnected"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CaseKey.self)
        guard let key = container.allKeys.first, container.allKeys.count == 1 else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: decoder.codingPath,
                debugDescription: "expected exactly one externally tagged AdapterEvent key"
            ))
        }
        switch key {
        case .stateChanged:
            self = .stateChanged(try container.decode(AdapterState.self, forKey: .stateChanged))
        case .deviceDiscovered:
            self = .deviceDiscovered(
                try container.decode(DiscoveredDevice.self, forKey: .deviceDiscovered))
        case .deviceUpdated:
            var fields = try container.nestedUnkeyedContainer(forKey: .deviceUpdated)
            let id = try fields.decode(DeviceId.self)
            let rssi = try fields.decode(Int16.self)
            self = .deviceUpdated(id, rssi: rssi)
        case .deviceConnected:
            self = .deviceConnected(
                try container.decode(DeviceId.self, forKey: .deviceConnected))
        case .deviceDisconnected:
            var fields = try container.nestedUnkeyedContainer(forKey: .deviceDisconnected)
            let id = try fields.decode(DeviceId.self)
            let reason = try fields.decodeIfPresent(DisconnectReason.self)
            self = .deviceDisconnected(id, reason)
        }
    }
}

/// The discovered GATT tree (`rblue_api::ServiceSet`).
struct ServiceSet: Decodable, Equatable, Sendable {
    let services: [Service]
}

/// One GATT service (`rblue_api::Service`).
struct Service: Decodable, Equatable, Sendable {
    let uuid: String
    let primary: Bool
    let characteristics: [Characteristic]
}

/// One GATT descriptor (`rblue_api::Descriptor`).
struct Descriptor: Decodable, Equatable, Sendable {
    let characteristic: CharacteristicId
    let uuid: String
    let instance: UInt16
}

/// One GATT characteristic (`rblue_api::Characteristic`).
struct Characteristic: Decodable, Equatable, Sendable {
    let uuid: String
    let properties: UInt16
    let descriptors: [Descriptor]
}

extension Characteristic {
    // GATT property bits (Bluetooth Core Spec, Vol 3 Part G 3.3.1.1).
    static let broadcastBit: UInt16 = 0x0001
    static let readBit: UInt16 = 0x0002
    static let writeWithoutResponseBit: UInt16 = 0x0004
    static let writeBit: UInt16 = 0x0008
    static let notifyBit: UInt16 = 0x0010
    static let indicateBit: UInt16 = 0x0020

    var canRead: Bool { properties & Self.readBit != 0 }
    var canWrite: Bool { properties & Self.writeBit != 0 }
    var canWriteWithoutResponse: Bool { properties & Self.writeWithoutResponseBit != 0 }
    var canNotify: Bool { properties & Self.notifyBit != 0 }
    var canIndicate: Bool { properties & Self.indicateBit != 0 }

    /// Compact property badges for the tree UI, e.g. `R W NR N`.
    var propertyBadges: [String] {
        var badges: [String] = []
        if canRead { badges.append("R") }
        if canWrite { badges.append("W") }
        if canWriteWithoutResponse { badges.append("NR") }
        if canNotify { badges.append("N") }
        if canIndicate { badges.append("I") }
        return badges
    }
}

/// Identity of a service within a connection (`rblue_api::ServiceId`).
struct ServiceIdJson: Codable, Equatable, Sendable {
    var uuid: String
    var instance: UInt16
}

/// Identity of a characteristic within one connected session
/// (`rblue_api::CharacteristicId`).
///
/// Session-scoped by contract: valid only for the current connection, never
/// persisted; after a reconnect fresh identifiers come from a new
/// `discover_services` call.
struct CharacteristicId: Codable, Equatable, Sendable {
    var service: ServiceIdJson
    var uuid: String
    var instance: UInt16
}

/// Scan filter request (`rblue_api::ScanFilter`).
///
/// `nil`/empty fields fall back to the facade's defaults through its
/// `parse_with_defaults` merge, so omitted keys mean "no filtering".
struct ScanFilter: Encodable, Sendable {
    var namePrefix: String?
    var services: [String]
    var rssiThreshold: Int16?

    enum CodingKeys: String, CodingKey {
        case namePrefix = "name_prefix"
        case services
        case rssiThreshold = "rssi_threshold"
    }
}

/// The adapter's capability set, parsed from the facade's `|`-separated
/// bitflags rendering (an empty set renders as `0x0`).
struct CapabilitySet: Equatable, Sendable {
    let raw: String
    let names: Set<String>

    static let knownNames: Set<String> = [
        "PASSIVE_SCAN", "ACTIVE_SCAN", "SCAN_RESPONSE", "GATT_CLIENT",
        "GATT_SERVER", "ADVERTISE", "RSSI_DURING_CONNECTION",
        "MTU_NEGOTIATION", "MTU_QUERY", "PAIRING", "WATCH_ADVERTISEMENTS",
        "REQUEST_DEVICE_PICKER", "BACKGROUND_SCAN",
    ]

    init(_ raw: String) {
        self.raw = raw
        let tokens = raw.split(separator: "|").map {
            $0.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        self.names = Set(tokens.filter { Self.knownNames.contains($0) })
    }

    func has(_ name: String) -> Bool {
        names.contains(name)
    }
}

extension Encodable {
    /// Canonical JSON accepted by the facade's JSON-string arguments.
    ///
    /// # Errors
    ///
    /// JSON encoding failures of the underlying type; for the contract types
    /// in this file these cannot occur in practice, so throwing call sites
    /// propagate into the surrounding error presentation instead of forcing
    /// a silent fallback.
    func jsonEncoded() throws -> String {
        let data = try JSONEncoder().encode(self)
        return String(decoding: data, as: UTF8.self)
    }
}

extension DeviceId {
    /// DeviceId JSON for facade method arguments, re-encoded from the parsed
    /// value (byte-identical to the Rust canonical form for all variants).
    func deviceJson() throws -> String {
        try jsonEncoded()
    }
}
