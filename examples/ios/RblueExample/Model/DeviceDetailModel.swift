import Foundation
import Rblue

/// State machine behind one device's detail screen: service discovery, the
/// GATT tree, read/write, the InvalidLength demonstration, and notification
/// subscriptions.
///
/// Discovery is mandatory on every connection (iOS requirement) and re-runs
/// after a reconnect, re-issuing the subscriptions the user kept (CCCD state
/// dies with the connection on CoreBluetooth).
@MainActor
final class DeviceDetailModel: ObservableObject {
    struct CharacteristicNode: Identifiable, Equatable {
        /// Canonical `CharacteristicId` JSON for facade calls.
        let id: String
        let serviceUuid: String
        let uuid: String
        let properties: UInt16
        let descriptorUuids: [String]
        var value: Data?
        var lastAction: String?
        var subscribed: Bool
    }

    struct ServiceNode: Identifiable, Equatable {
        let id: String
        let uuid: String
        let primary: Bool
        var characteristics: [CharacteristicNode]
    }

    let device: DeviceId

    @Published private(set) var services: [ServiceNode] = []
    @Published private(set) var rssi: Int16?
    @Published private(set) var deviceName: String?
    @Published private(set) var isDiscovering = false
    @Published private(set) var busyCharacteristicIds: Set<String> = []
    @Published private(set) var presentedError: PresentedError?

    private let client: RblueClient
    private var facade: (any RblueFacade)?
    /// Characteristic ids (canonical JSON) the user subscribed; survives
    /// rediscovery so subscriptions are re-issued after a reconnect.
    private var desiredSubscriptions: Set<String> = []

    private static let decoder = JSONDecoder()

    init(device: DeviceId, client: RblueClient) {
        self.device = device
        self.client = client
        self.facade = client.activeFacade
    }

    var isConnected: Bool {
        client.isConnected(device)
    }

    var notifications: [Notification] {
        client.notificationLogs[device] ?? []
    }

    // MARK: - Discovery

    /// Ensures the link is connected before discovering the session-scoped
    /// GATT tree. Opening a device row uses this path on real hardware and in
    /// the simulator demo.
    func prepare() async {
        if !isConnected {
            await client.connect(device)
            for _ in 0..<200 where !isConnected {
                try? await Task.sleep(for: .milliseconds(10))
            }
        }
        await discover()
        if RblueClient.demoDetailEnabled {
            await runDemoActions()
        }
    }

    /// Demo-only: read, subscribe, and write Echo so the details screen shows
    /// data and a notification without desktop UI automation permissions.
    private func runDemoActions() async {
        guard
            let node = services
                .flatMap(\.characteristics)
                .first(where: { $0.uuid == DemoRblueFacade.echoCharacteristic })
        else { return }
        await read(node)
        await toggleSubscription(node)
        await write(node, payload: Data("Ping from simulator".utf8), withResponse: true)
    }

    /// Discovers the full GATT tree (CoreBluetooth has no filtered
    /// discovery; the facade filters in software when service UUIDs are
    /// given - the demo always discovers everything), reads the connected
    /// RSSI, and re-issues surviving subscriptions.
    func discover() async {
        guard let facade, isConnected, !isDiscovering else { return }
        isDiscovering = true
        defer { isDiscovering = false }
        do {
            let deviceJson = try device.deviceJson()
            let treeJson = try await facade.discoverServices(
                deviceJson: deviceJson, serviceUuids: [])
            let set = try Self.decoder.decode(ServiceSet.self, from: Data(treeJson.utf8))
            services = try Self.buildTree(from: set)
            for node in services.flatMap(\.characteristics)
            where desiredSubscriptions.contains(node.id) {
                try await facade.subscribe(
                    deviceJson: deviceJson, characteristicJson: node.id)
                mutateCharacteristic(node.id) { $0.subscribed = true }
            }
            if !desiredSubscriptions.isEmpty {
                client.registerNotificationTarget(device)
            }
            rssi = try await facade.rssi(deviceJson: deviceJson)
            // `name` is a non-throwing facade method (None for unknown
            // devices); the scan row already showed the advertised name.
            deviceName = await facade.name(deviceJson: deviceJson)
        } catch {
            presentedError = .from(error)
        }
    }

    /// Builds the service/characteristic tree from a `ServiceSet`,
    /// reconstructing the session-scoped `CharacteristicId`s (instance =
    /// zero-based index among same-UUID entries in discovery order).
    private static func buildTree(from set: ServiceSet) throws -> [ServiceNode] {
        var nodes: [ServiceNode] = []
        nodes.reserveCapacity(set.services.count)
        var serviceInstances: [String: UInt16] = [:]
        for service in set.services {
            let serviceInstance = serviceInstances[service.uuid, default: 0]
            serviceInstances[service.uuid, default: 0] += 1
            var characteristicNodes: [CharacteristicNode] = []
            characteristicNodes.reserveCapacity(service.characteristics.count)
            var characteristicInstances: [String: UInt16] = [:]
            for characteristic in service.characteristics {
                let characteristicInstance = characteristicInstances[
                    characteristic.uuid, default: 0]
                characteristicInstances[characteristic.uuid, default: 0] += 1
                let identifier = CharacteristicId(
                    service: ServiceIdJson(uuid: service.uuid, instance: serviceInstance),
                    uuid: characteristic.uuid,
                    instance: characteristicInstance
                )
                characteristicNodes.append(CharacteristicNode(
                    id: try identifier.jsonEncoded(),
                    serviceUuid: service.uuid,
                    uuid: characteristic.uuid,
                    properties: characteristic.properties,
                    descriptorUuids: characteristic.descriptors.map(\.uuid),
                    value: nil,
                    lastAction: nil,
                    subscribed: false
                ))
            }
            nodes.append(ServiceNode(
                id: "\(service.uuid)#\(serviceInstance)",
                uuid: service.uuid,
                primary: service.primary,
                characteristics: characteristicNodes
            ))
        }
        return nodes
    }

    // MARK: - GATT operations

    /// Reads the characteristic's current value.
    func read(_ node: CharacteristicNode) async {
        guard let facade else { return }
        await withBusy(node.id) {
            let data = try await facade.read(
                deviceJson: try self.device.deviceJson(), characteristicJson: node.id)
            self.mutateCharacteristic(node.id) {
                $0.value = data
                $0.lastAction = "读取 \(data.count) 字节"
            }
        }
    }

    /// Writes the payload in the requested mode. `max_write_len` is
    /// re-queried per write instead of cached (the MTU is renegotiated per
    /// connection and can change mid-session). Oversize payloads are
    /// rejected by the facade with the structured `InvalidLength` error.
    func write(_ node: CharacteristicNode, payload: Data, withResponse: Bool) async {
        guard let facade else { return }
        await withBusy(node.id) {
            let deviceJson = try self.device.deviceJson()
            let maxLen = try await facade.maxWriteLen(
                deviceJson: deviceJson, characteristicJson: node.id, withResponse: withResponse)
            try await facade.write(
                deviceJson: deviceJson, characteristicJson: node.id, data: payload,
                withResponse: withResponse)
            self.mutateCharacteristic(node.id) {
                $0.value = payload
                $0.lastAction = "已写入 \(payload.count) 字节 "
                    + "(\(withResponse ? "with response" : "without response"), "
                    + "max_write_len \(maxLen))"
            }
        }
    }

    /// The InvalidLength demonstration: writes a payload one byte over
    /// `max_write_len`, expecting the structured rejection. Native
    /// CoreBluetooth would silently truncate a without-response write; rblue
    /// never truncates, and this makes the platform difference visible.
    func writeOversizeDemo(_ node: CharacteristicNode, withResponse: Bool) async {
        guard let facade else { return }
        await withBusy(node.id) {
            let deviceJson = try self.device.deviceJson()
            let maxLen = try await facade.maxWriteLen(
                deviceJson: deviceJson, characteristicJson: node.id, withResponse: withResponse)
            let payload = Data(repeating: 0xAB, count: Int(maxLen) + 1)
            do {
                try await facade.write(
                    deviceJson: deviceJson, characteristicJson: node.id, data: payload,
                    withResponse: withResponse)
                self.presentedError = PresentedError(
                    title: "演示异常",
                    detail: "超长载荷(len=\(payload.count) > max=\(maxLen))没有被拒绝,"
                        + "违反统一写语义(InvalidLength),请上报。",
                    structured: "write accepted len=\(payload.count) max=\(maxLen)"
                )
            } catch {
                // Expected path: the structured InvalidLength surfaces here.
                self.presentedError = .from(error)
                self.mutateCharacteristic(node.id) {
                    $0.lastAction = "InvalidLength 演示:载荷 \(payload.count) 字节 > max \(maxLen)"
                }
            }
        }
    }

    /// Subscribes to value changes / unsubscribes; notification vs.
    /// indication follows the characteristic's properties. Values arrive
    /// through the client's notification log.
    func toggleSubscription(_ node: CharacteristicNode) async {
        guard let facade else { return }
        await withBusy(node.id) {
            let deviceJson = try self.device.deviceJson()
            if self.desiredSubscriptions.contains(node.id) {
                try await facade.unsubscribe(
                    deviceJson: deviceJson, characteristicJson: node.id)
                self.desiredSubscriptions.remove(node.id)
                self.mutateCharacteristic(node.id) {
                    $0.subscribed = false
                    $0.lastAction = "已取消订阅"
                }
                if self.desiredSubscriptions.isEmpty {
                    self.client.unregisterNotificationTarget(self.device)
                }
            } else {
                try await facade.subscribe(
                    deviceJson: deviceJson, characteristicJson: node.id)
                self.desiredSubscriptions.insert(node.id)
                self.client.registerNotificationTarget(self.device)
                self.mutateCharacteristic(node.id) {
                    $0.subscribed = true
                    $0.lastAction = "已订阅,值将通过通知到达"
                }
            }
        }
    }

    /// Cancels remaining subscriptions when the detail view goes away. A
    /// disconnected device needs no work (its CCCD state died with the
    /// link); while connected, failures surface as errors.
    func unsubscribeAll() async {
        guard let facade, isConnected, !desiredSubscriptions.isEmpty else { return }
        do {
            let deviceJson = try device.deviceJson()
            for id in desiredSubscriptions {
                try await facade.unsubscribe(
                    deviceJson: deviceJson, characteristicJson: id)
            }
            desiredSubscriptions = []
            for service in services.indices {
                for characteristic in services[service].characteristics.indices {
                    services[service].characteristics[characteristic].subscribed = false
                }
            }
            client.unregisterNotificationTarget(device)
        } catch {
            presentedError = .from(error)
        }
    }

    // MARK: - Platform-limitation probes

    /// Probes `mtu()` - expected to fail with `Unsupported` on iOS/macOS
    /// (the negotiated MTU is not exposed; derive limits from
    /// `max_write_len`). An unexpected success is surfaced as a bug.
    func probeMtu() async {
        guard let facade else { return }
        do {
            let value = try await facade.mtu(deviceJson: try device.deviceJson())
            presentedError = PresentedError(
                title: "演示异常",
                detail: "mtu() 返回了 \(value);按平台事实 iOS 不暴露 MTU,应返回 Unsupported,请上报。",
                structured: "mtu=\(value)"
            )
        } catch {
            presentedError = .from(error)
        }
    }

    /// Probes `pair()` - expected `Unsupported` (pairing is implicit on iOS,
    /// triggered by access to an encrypted characteristic).
    func probePair() async {
        guard let facade else { return }
        do {
            try await facade.pair(deviceJson: try device.deviceJson())
            presentedError = PresentedError(
                title: "演示异常",
                detail: "pair() 成功了;按平台事实 iOS 配对是隐式的,应返回 Unsupported,请上报。",
                structured: "pair accepted"
            )
        } catch {
            presentedError = .from(error)
        }
    }

    // MARK: - Plumbing

    func dismissError() {
        presentedError = nil
    }

    private func withBusy(_ characteristicId: String, _ body: () async throws -> Void) async {
        busyCharacteristicIds.insert(characteristicId)
        defer { busyCharacteristicIds.remove(characteristicId) }
        do {
            try await body()
        } catch {
            presentedError = .from(error)
        }
    }

    private func mutateCharacteristic(
        _ id: String, _ mutate: (inout CharacteristicNode) -> Void
    ) {
        for serviceIndex in services.indices {
            if let characteristicIndex = services[serviceIndex].characteristics.firstIndex(
                where: { $0.id == id }
            ) {
                mutate(&services[serviceIndex].characteristics[characteristicIndex])
                return
            }
        }
    }
}

extension DeviceDetailModel.CharacteristicNode {
    // GATT property accessors mirroring `Characteristic`.
    var canRead: Bool { properties & Characteristic.readBit != 0 }
    var canWrite: Bool { properties & Characteristic.writeBit != 0 }
    var canWriteWithoutResponse: Bool {
        properties & Characteristic.writeWithoutResponseBit != 0
    }
    var canNotify: Bool { properties & Characteristic.notifyBit != 0 }
    var canIndicate: Bool { properties & Characteristic.indicateBit != 0 }

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

/// Byte-payload rendering helpers for the tree UI.
enum DataRendering {
    static func hex(_ data: Data) -> String {
        data.map { String(format: "%02x", $0) }.joined(separator: " ")
    }

    static func text(_ data: Data) -> String {
        String(decoding: data, as: UTF8.self)
    }
}
