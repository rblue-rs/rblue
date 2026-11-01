import XCTest

import Rblue
@testable import RblueExample

/// In-memory facade for client-flow tests: records calls, replays scripted
/// outcomes. The client talks to `RblueFacade`, so no hardware or xcframework
/// backend is exercised here (the simulator has no Bluetooth anyway).
private final class FakeRblue: RblueFacade, @unchecked Sendable {
    private(set) var installedSink: EventSink?
    private(set) var initializeCalls = 0
    private(set) var startScanJsons: [String] = []
    private(set) var stopScanCalls = 0
    private(set) var connectJsons: [String] = []
    private(set) var disconnectJsons: [String] = []
    private(set) var subscribeJsons: [String] = []
    private(set) var unsubscribeJsons: [String] = []

    /// Scripted behavior for `connect`; failures throw `RblueError`.
    var connectHandler: ((String) async throws -> Void)?

    func initialize() async throws {
        initializeCalls += 1
    }

    func adapterId() -> String { "fake-adapter" }

    func backendVersion() -> String { "fake-backend-1.0" }

    func capabilities() -> String { "ACTIVE_SCAN | GATT_CLIENT" }

    func startScan(filterJson: String) async throws {
        startScanJsons.append(filterJson)
    }

    func stopScan() async throws {
        stopScanCalls += 1
    }

    func connect(deviceJson: String) async throws {
        connectJsons.append(deviceJson)
        try await connectHandler?(deviceJson)
    }

    func disconnect(deviceJson: String) async throws {
        disconnectJsons.append(deviceJson)
    }

    func discoverServices(
        deviceJson: String, serviceUuids: [String]?
    ) async throws -> String {
        #"{"services":[]}"#
    }

    func read(deviceJson: String, characteristicJson: String) async throws -> Data {
        Data()
    }

    func write(
        deviceJson: String, characteristicJson: String, data: Data, withResponse: Bool
    ) async throws {}

    func maxWriteLen(
        deviceJson: String, characteristicJson: String, withResponse: Bool
    ) async throws -> UInt64 {
        20
    }

    func subscribe(deviceJson: String, characteristicJson: String) async throws {
        subscribeJsons.append(characteristicJson)
    }

    func unsubscribe(deviceJson: String, characteristicJson: String) async throws {
        unsubscribeJsons.append(characteristicJson)
    }

    func rssi(deviceJson: String) async throws -> Int16 {
        -42
    }

    func mtu(deviceJson: String) async throws -> UInt16 {
        throw RblueError.Unsupported(platform: "ios", feature: "mtu()")
    }

    func name(deviceJson: String) async -> String? {
        nil
    }

    func pair(deviceJson: String) async throws {
        throw RblueError.Unsupported(platform: "ios", feature: "pair()")
    }

    func setEventSink(sink: EventSink) {
        installedSink = sink
    }
}

@MainActor
final class ClientFlowTests: XCTestCase {
    private let fixtureUuid = "4c8853a7-7d4e-4997-9c58-344b9e5e757f"

    /// Polls a condition until it holds or the timeout elapses; the event
    /// pump delivers asynchronously, so immediate assertions would race.
    private func waitUntil(
        timeout: TimeInterval = 2,
        _ condition: () -> Bool,
        _ failure: String
    ) async {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() > deadline {
                XCTFail(failure)
                return
            }
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    func testStartInstallsEventSinkAndSurfacesDiagnostics() {
        let fake = FakeRblue()
        let client = RblueClient(facadeFactory: { fake })
        client.start()
        XCTAssertNotNil(fake.installedSink)
        XCTAssertEqual(client.backendVersion, "fake-backend-1.0")
        XCTAssertEqual(client.adapterId, "fake-adapter")
        XCTAssertTrue(client.capabilities.has("GATT_CLIENT"))
    }

    func testEventsFlowIntoClientState() async {
        let fake = FakeRblue()
        let client = RblueClient(facadeFactory: { fake })
        client.start()
        guard let sink = fake.installedSink else {
            return XCTFail("sink must be installed by start()")
        }

        sink.onEvent(eventJson: #"{"state_changed":"powered_on"}"#)
        await waitUntil({ client.adapterState == .poweredOn }, "state_changed not applied")

        sink.onEvent(
            eventJson:
                #"{"device_discovered":{"advertisement":{"local_name":"Echo","services":[],"service_data":[],"manufacturer_data":null,"tx_power":null,"adv_flags":null,"rssi":-42,"is_connectable":true},"id":{"uuid":"\#(fixtureUuid)"}}}"#
        )
        await waitUntil(
            { client.scan.entries.count == 1 },
            "device_discovered not applied"
        )
        XCTAssertEqual(client.scan.entries[.uuid(fixtureUuid)]?.name, "Echo")

        sink.onEvent(eventJson: #"{"device_updated":[{"uuid":"\#(fixtureUuid)"},-33]}"#)
        await waitUntil(
            { client.scan.entries[.uuid(fixtureUuid)]?.rssi == -33 },
            "device_updated not applied"
        )
    }

    func testScanStartSendsFilterJsonAndStops() async throws {
        let fake = FakeRblue()
        let client = RblueClient(facadeFactory: { fake })
        client.start()
        await client.startScan(namePrefix: "Echo", serviceUuid: "180F", rssiThreshold: -80)
        XCTAssertEqual(fake.startScanJsons.count, 1)
        XCTAssertEqual(
            try TestJSON.object(try XCTUnwrap(fake.startScanJsons.first)),
            ["name_prefix": "Echo", "services": ["180F"], "rssi_threshold": -80]
                as NSDictionary
        )
        await client.stopScan()
        XCTAssertEqual(fake.stopScanCalls, 1)
    }

    /// Failure visibility: the first reconnect attempt fails with a
    /// structured platform error; the long base delay keeps the recorded
    /// error observable before the loop's next attempt.
    func testReconnectFailureKeepsStructuredPlatformError() async {
        let fake = FakeRblue()
        var connectCalls = 0
        fake.connectHandler = { _ in
            connectCalls += 1
            if connectCalls == 2 {
                throw RblueError.Platform(code: 133, detail: "simulated link failure")
            }
        }
        let client = RblueClient(facadeFactory: { fake }, reconnectBaseMilliseconds: 300)
        client.start()
        guard let sink = fake.installedSink else {
            return XCTFail("sink must be installed by start()")
        }

        await client.connect(.uuid(fixtureUuid))
        sink.onEvent(eventJson: #"{"device_connected":{"uuid":"\#(fixtureUuid)"}}"#)
        await waitUntil(
            { client.isConnected(.uuid(fixtureUuid)) },
            "device_connected not applied"
        )

        // Unexpected drop (the user never disconnected) must schedule the
        // reconnect loop.
        sink.onEvent(eventJson: #"{"device_disconnected":[{"uuid":"\#(fixtureUuid)"},"unknown"]}"#)
        await waitUntil(
            { fake.connectJsons.count >= 2 },
            "reconnect attempt not made"
        )
        // The failed attempt's structured error persists on the banner while
        // the loop backs off (~600 ms before the next attempt here).
        await waitUntil(
            {
                client.reconnectState(for: .uuid(fixtureUuid))?
                    .lastError?.contains("133") == true
            },
            "failed attempt must record the structured platform error"
        )

        // The (manually replayed) connected event cancels the loop and
        // clears the reconnect state.
        sink.onEvent(eventJson: #"{"device_connected":{"uuid":"\#(fixtureUuid)"}}"#)
        await waitUntil(
            {
                client.isConnected(.uuid(fixtureUuid))
                    && client.reconnectState(for: .uuid(fixtureUuid)) == nil
            },
            "successful reconnect must clear the reconnect state"
        )
    }

    /// Success path: with a short base delay the second attempt succeeds and
    /// the connected event clears the reconnect bookkeeping.
    func testReconnectSucceedsOnSecondAttempt() async {
        let fake = FakeRblue()
        var connectCalls = 0
        fake.connectHandler = { _ in
            connectCalls += 1
            if connectCalls == 2 {
                throw RblueError.Platform(code: 133, detail: "simulated link failure")
            }
        }
        let client = RblueClient(facadeFactory: { fake }, reconnectBaseMilliseconds: 5)
        client.start()
        guard let sink = fake.installedSink else {
            return XCTFail("sink must be installed by start()")
        }

        await client.connect(.uuid(fixtureUuid))
        sink.onEvent(eventJson: #"{"device_connected":{"uuid":"\#(fixtureUuid)"}}"#)
        await waitUntil(
            { client.isConnected(.uuid(fixtureUuid)) },
            "device_connected not applied"
        )
        sink.onEvent(eventJson: #"{"device_disconnected":[{"uuid":"\#(fixtureUuid)"},"unknown"]}"#)
        await waitUntil(
            { fake.connectJsons.count >= 3 },
            "second reconnect attempt not made"
        )
        sink.onEvent(eventJson: #"{"device_connected":{"uuid":"\#(fixtureUuid)"}}"#)
        await waitUntil(
            {
                client.isConnected(.uuid(fixtureUuid))
                    && client.reconnectState(for: .uuid(fixtureUuid)) == nil
            },
            "reconnect bookkeeping must clear after success"
        )
        XCTAssertEqual(fake.connectJsons.count, 3)
    }

    func testUserDisconnectDoesNotReconnect() async {
        let fake = FakeRblue()
        let client = RblueClient(facadeFactory: { fake }, reconnectBaseMilliseconds: 5)
        client.start()
        guard let sink = fake.installedSink else {
            return XCTFail("sink must be installed by start()")
        }

        await client.connect(.uuid(fixtureUuid))
        sink.onEvent(eventJson: #"{"device_connected":{"uuid":"\#(fixtureUuid)"}}"#)
        await waitUntil(
            { client.isConnected(.uuid(fixtureUuid)) }, "device_connected not applied"
        )

        await client.disconnect(.uuid(fixtureUuid))
        XCTAssertEqual(fake.disconnectJsons, ["{\"uuid\":\"\(fixtureUuid)\"}"])
        sink.onEvent(eventJson: #"{"device_disconnected":[{"uuid":"\#(fixtureUuid)"},"local"]}"#)
        await waitUntil(
            { client.isConnected(.uuid(fixtureUuid)) == false },
            "disconnect event not applied"
        )
        // Give a would-be reconnect loop time to misfire before asserting.
        try? await Task.sleep(for: .milliseconds(150))
        XCTAssertEqual(fake.connectJsons.count, 1, "user disconnect must not reconnect")
    }

    func testNotificationsLandInRegisteredTargetsLog() async {
        let fake = FakeRblue()
        let client = RblueClient(facadeFactory: { fake })
        client.start()
        guard let sink = fake.installedSink else {
            return XCTFail("sink must be installed by start()")
        }
        client.registerNotificationTarget(.uuid(fixtureUuid))

        sink.onNotification(
            notificationJson:
                #"{"id":{"service":{"uuid":"180f","instance":0},"uuid":"2a19","instance":0},"data":[104,105]}"#
        )
        await waitUntil(
            { client.notificationLogs[.uuid(fixtureUuid)]?.count == 1 },
            "notification not attributed"
        )
        XCTAssertEqual(
            client.notificationLogs[.uuid(fixtureUuid)]?.first?.data, [104, 105]
        )

        // Unregistered targets receive nothing.
        client.unregisterNotificationTarget(.uuid(fixtureUuid))
        sink.onNotification(
            notificationJson:
                #"{"id":{"service":{"uuid":"180f","instance":0},"uuid":"2a19","instance":0},"data":[111]}"#
        )
        try? await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(
            client.notificationLogs[.uuid(fixtureUuid)]?.count, 1,
            "unregistered target must not receive notifications"
        )
    }
}
