import Foundation
import Rblue

/// Simulator-only, in-process BLE facade used by the `--demo` launch mode.
///
/// It speaks the exact JSON and error contracts of `rblue-ffi`, so the regular
/// scan/detail views exercise their real parsing, state, read/write,
/// notification, InvalidLength, and reconnect code without requiring a radio.
/// The normal simulator launch does not use this facade and still reports the
/// truthful "simulators have no BLE hardware" Unsupported error.
final class DemoRblueFacade: RblueFacade, @unchecked Sendable {
    static let echoDevice = "00000000-0000-4000-8000-00000000e001"
    static let batteryDevice = "00000000-0000-4000-8000-00000000b001"

    static let echoService = "0000ffe0-0000-1000-8000-00805f9b34fb"
    static let echoCharacteristic = "0000ffe1-0000-1000-8000-00805f9b34fb"
    static let batteryService = "0000180f-0000-1000-8000-00805f9b34fb"
    static let batteryCharacteristic = "00002a19-0000-1000-8000-00805f9b34fb"

    private struct DemoDevice {
        let id: String
        let name: String
        let rssi: Int16
        let services: [String]
        let manufacturerData: [UInt8]
    }

    private struct CharacteristicRequest: Decodable {
        let uuid: String
    }

    private let lock = NSLock()
    private var sink: EventSink?
    private var scanning = false
    private var connected: Set<String> = []
    private var subscribed: Set<String> = []
    private var values: [String: Data] = [
        echoCharacteristic: Data("Hello from rblue demo".utf8),
        batteryCharacteristic: Data([87]),
    ]

    private let devices = [
        DemoDevice(
            id: echoDevice,
            name: "Echo Sensor",
            rssi: -38,
            services: [echoService],
            manufacturerData: [0x52, 0x42, 0x4c, 0x55, 0x45]
        ),
        DemoDevice(
            id: batteryDevice,
            name: "Battery Tag",
            rssi: -61,
            services: [batteryService],
            manufacturerData: [0x57]
        ),
    ]

    func initialize() async throws {
        emitEvent(["state_changed": "powered_on"])
    }

    func adapterId() -> String { "ios-simulator-demo" }

    func backendVersion() -> String { "demo/ios-simulator" }

    func capabilities() -> String {
        "ACTIVE_SCAN | GATT_CLIENT | RSSI_DURING_CONNECTION"
    }

    func startScan(filterJson: String) async throws {
        let filter = try Self.jsonObject(filterJson)
        let namePrefix = filter["name_prefix"] as? String
        let services = filter["services"] as? [String] ?? []
        let rssiThreshold = (filter["rssi_threshold"] as? NSNumber)?.int16Value
        lock.lock()
        scanning = true
        lock.unlock()

        for device in devices where Self.matches(
            device,
            namePrefix: namePrefix,
            services: services,
            rssiThreshold: rssiThreshold
        ) {
            emitEvent([
                "device_discovered": [
                    "id": ["uuid": device.id],
                    "advertisement": [
                        "local_name": device.name,
                        "services": device.services,
                        "service_data": [],
                        "manufacturer_data": [76, device.manufacturerData],
                        "tx_power": NSNull(),
                        "adv_flags": NSNull(),
                        "rssi": device.rssi,
                        "is_connectable": true,
                    ],
                ],
            ])
        }
    }

    func stopScan() async throws {
        lock.lock()
        scanning = false
        lock.unlock()
    }

    func connect(deviceJson: String) async throws {
        let id = try Self.deviceId(deviceJson)
        guard devices.contains(where: { $0.id == id }) else {
            throw RblueError.NotFound(device: deviceJson)
        }
        lock.lock()
        connected.insert(id)
        lock.unlock()
        emitEvent(["device_connected": ["uuid": id]])
    }

    func disconnect(deviceJson: String) async throws {
        let id = try Self.deviceId(deviceJson)
        lock.lock()
        connected.remove(id)
        subscribed = subscribed.filter { !$0.hasPrefix("\(id)|") }
        lock.unlock()
        emitEvent(["device_disconnected": [["uuid": id], "local"]])
    }

    func discoverServices(deviceJson: String, serviceUuids: [String]?) async throws -> String {
        let id = try Self.connectedDevice(deviceJson, connected: connectedSnapshot())
        let services: [[String: Any]]
        switch id {
        case Self.echoDevice:
            services = [[
                "uuid": Self.echoService,
                "primary": true,
                "characteristics": [[
                    "uuid": Self.echoCharacteristic,
                    "properties": 30, // READ | WRITE_NR | WRITE | NOTIFY
                    "descriptors": ["00002902-0000-1000-8000-00805f9b34fb"],
                ]],
            ]]
        case Self.batteryDevice:
            services = [[
                "uuid": Self.batteryService,
                "primary": true,
                "characteristics": [[
                    "uuid": Self.batteryCharacteristic,
                    "properties": 18, // READ | NOTIFY
                    "descriptors": ["00002902-0000-1000-8000-00805f9b34fb"],
                ]],
            ]]
        default:
            throw RblueError.NotFound(device: deviceJson)
        }
        return try Self.jsonString(["services": services])
    }

    func read(deviceJson: String, characteristicJson: String) async throws -> Data {
        _ = try Self.connectedDevice(deviceJson, connected: connectedSnapshot())
        let uuid = try Self.characteristicUuid(characteristicJson)
        lock.lock()
        defer { lock.unlock() }
        guard let value = values[uuid] else {
            throw RblueError.Att(code: 0x01, handle: nil)
        }
        return value
    }

    func write(
        deviceJson: String,
        characteristicJson: String,
        data: Data,
        withResponse: Bool
    ) async throws {
        let id = try Self.connectedDevice(deviceJson, connected: connectedSnapshot())
        let uuid = try Self.characteristicUuid(characteristicJson)
        let max = try await maxWriteLen(
            deviceJson: deviceJson,
            characteristicJson: characteristicJson,
            withResponse: withResponse
        )
        guard data.count <= max else {
            throw RblueError.InvalidLength(len: UInt64(data.count), max: UInt64(max))
        }
        guard uuid == Self.echoCharacteristic else {
            throw RblueError.Att(code: 0x03, handle: nil)
        }
        lock.lock()
        values[uuid] = data
        let shouldNotify = subscribed.contains(Self.subscriptionKey(device: id, characteristic: uuid))
        lock.unlock()
        if shouldNotify {
            emitNotification(characteristicJson: characteristicJson, data: data)
        }
    }

    func maxWriteLen(
        deviceJson: String,
        characteristicJson: String,
        withResponse: Bool
    ) async throws -> UInt64 {
        _ = try Self.connectedDevice(deviceJson, connected: connectedSnapshot())
        let uuid = try Self.characteristicUuid(characteristicJson)
        guard uuid == Self.echoCharacteristic else {
            throw RblueError.Att(code: 0x03, handle: nil)
        }
        return 20
    }

    func subscribe(deviceJson: String, characteristicJson: String) async throws {
        let id = try Self.connectedDevice(deviceJson, connected: connectedSnapshot())
        let uuid = try Self.characteristicUuid(characteristicJson)
        guard uuid == Self.echoCharacteristic || uuid == Self.batteryCharacteristic else {
            throw RblueError.Att(code: 0x03, handle: nil)
        }
        lock.lock()
        subscribed.insert(Self.subscriptionKey(device: id, characteristic: uuid))
        let value = values[uuid] ?? Data()
        lock.unlock()
        emitNotification(characteristicJson: characteristicJson, data: value)
    }

    func unsubscribe(deviceJson: String, characteristicJson: String) async throws {
        let id = try Self.deviceId(deviceJson)
        let uuid = try Self.characteristicUuid(characteristicJson)
        lock.lock()
        subscribed.remove(Self.subscriptionKey(device: id, characteristic: uuid))
        lock.unlock()
    }

    func rssi(deviceJson: String) async throws -> Int16 {
        let id = try Self.connectedDevice(deviceJson, connected: connectedSnapshot())
        guard let device = devices.first(where: { $0.id == id }) else {
            throw RblueError.NotFound(device: deviceJson)
        }
        return device.rssi
    }

    func mtu(deviceJson: String) async throws -> UInt16 {
        throw RblueError.Unsupported(platform: "ios", feature: "mtu()")
    }

    func name(deviceJson: String) async -> String? {
        guard let id = try? Self.deviceId(deviceJson) else { return nil }
        return devices.first(where: { $0.id == id })?.name
    }

    func pair(deviceJson: String) async throws {
        throw RblueError.Unsupported(platform: "ios", feature: "pair()")
    }

    func setEventSink(sink: EventSink) {
        lock.lock()
        self.sink = sink
        lock.unlock()
    }

    private func connectedSnapshot() -> Set<String> {
        lock.lock()
        defer { lock.unlock() }
        return connected
    }

    private func currentSink() -> EventSink? {
        lock.lock()
        defer { lock.unlock() }
        return sink
    }

    private func emitEvent(_ object: Any) {
        guard let sink = currentSink(), let json = try? Self.jsonString(object) else { return }
        sink.onEvent(eventJson: json)
    }

    private func emitNotification(characteristicJson: String, data: Data) {
        guard
            let sink = currentSink(),
            let id = try? Self.jsonObject(characteristicJson),
            let json = try? Self.jsonString(["id": id, "data": [UInt8](data)])
        else { return }
        sink.onNotification(notificationJson: json)
    }

    private static func matches(
        _ device: DemoDevice,
        namePrefix: String?,
        services: [String],
        rssiThreshold: Int16?
    ) -> Bool {
        if let namePrefix, !namePrefix.isEmpty,
           !device.name.lowercased().hasPrefix(namePrefix.lowercased()) {
            return false
        }
        if !services.isEmpty {
            let wanted = Set(services.map { $0.lowercased() })
            if wanted.isDisjoint(with: device.services.map { $0.lowercased() }) {
                return false
            }
        }
        if let rssiThreshold, device.rssi < rssiThreshold { return false }
        return true
    }

    private static func connectedDevice(_ json: String, connected: Set<String>) throws -> String {
        let id = try deviceId(json)
        guard connected.contains(id) else { throw RblueError.NotConnected }
        return id
    }

    private static func deviceId(_ json: String) throws -> String {
        let object = try jsonObject(json)
        guard let uuid = object["uuid"] as? String else {
            throw RblueError.InvalidArgument(detail: "demo expects DeviceId::Uuid")
        }
        return uuid
    }

    private static func characteristicUuid(_ json: String) throws -> String {
        let request = try JSONDecoder().decode(
            CharacteristicRequest.self, from: Data(json.utf8))
        return request.uuid
    }

    private static func subscriptionKey(device: String, characteristic: String) -> String {
        "\(device)|\(characteristic)"
    }

    private static func jsonObject(_ json: String) throws -> [String: Any] {
        guard
            let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
                as? [String: Any]
        else {
            throw RblueError.InvalidArgument(detail: "expected JSON object")
        }
        return object
    }

    private static func jsonString(_ object: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }
}
