import Foundation
import XCTest

import Rblue
@testable import RblueExample

private final class RecordingDemoSink: EventSink, @unchecked Sendable {
    private let lock = NSLock()
    private(set) var events: [String] = []
    private(set) var notifications: [String] = []

    func onEvent(eventJson: String) {
        lock.lock()
        events.append(eventJson)
        lock.unlock()
    }

    func onNotification(notificationJson: String) {
        lock.lock()
        notifications.append(notificationJson)
        lock.unlock()
    }
}

final class DemoRblueFacadeTests: XCTestCase {
    private let echoDevice = #"{"uuid":"00000000-0000-4000-8000-00000000e001"}"#
    private let echoCharacteristic =
        #"{"service":{"uuid":"0000ffe0-0000-1000-8000-00805f9b34fb","instance":0},"uuid":"0000ffe1-0000-1000-8000-00805f9b34fb","instance":0}"#

    func testInitializeAndScanEmitStateAndTwoDiscoveries() async throws {
        let facade = DemoRblueFacade()
        let sink = RecordingDemoSink()
        facade.setEventSink(sink: sink)

        try await facade.initialize()
        try await facade.startScan(filterJson: #"{"services":[]}"#)

        XCTAssertEqual(sink.events.count, 3)
        XCTAssertTrue(sink.events[0].contains("state_changed"))
        XCTAssertTrue(sink.events[1].contains("Echo Sensor"))
        XCTAssertTrue(sink.events[2].contains("Battery Tag"))
    }

    func testConnectDiscoverReadWriteAndInvalidLength() async throws {
        let facade = DemoRblueFacade()
        let sink = RecordingDemoSink()
        facade.setEventSink(sink: sink)
        try await facade.connect(deviceJson: echoDevice)

        let tree = try await facade.discoverServices(
            deviceJson: echoDevice,
            serviceUuids: []
        )
        XCTAssertTrue(tree.contains(DemoRblueFacade.echoService))
        XCTAssertTrue(tree.contains(DemoRblueFacade.echoCharacteristic))

        let initial = try await facade.read(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic
        )
        XCTAssertEqual(String(decoding: initial, as: UTF8.self), "Hello from rblue demo")

        let payload = Data("ping".utf8)
        try await facade.write(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic,
            data: payload,
            withResponse: true
        )
        let readBack = try await facade.read(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic
        )
        XCTAssertEqual(readBack, payload)

        do {
            try await facade.write(
                deviceJson: echoDevice,
                characteristicJson: echoCharacteristic,
                data: Data(repeating: 0x41, count: 21),
                withResponse: true
            )
            XCTFail("oversize write must fail")
        } catch let RblueError.InvalidLength(len, max) {
            XCTAssertEqual(len, 21)
            XCTAssertEqual(max, 20)
        }
    }

    func testSubscribeDeliversAndUnsubscribeStopsNotifications() async throws {
        let facade = DemoRblueFacade()
        let sink = RecordingDemoSink()
        facade.setEventSink(sink: sink)
        try await facade.connect(deviceJson: echoDevice)
        try await facade.subscribe(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic
        )
        XCTAssertEqual(sink.notifications.count, 1)

        try await facade.write(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic,
            data: Data("notify".utf8),
            withResponse: true
        )
        XCTAssertEqual(sink.notifications.count, 2)
        XCTAssertTrue(sink.notifications.last?.contains("110,111,116,105,102,121") == true)

        try await facade.unsubscribe(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic
        )
        try await facade.write(
            deviceJson: echoDevice,
            characteristicJson: echoCharacteristic,
            data: Data("silent".utf8),
            withResponse: true
        )
        XCTAssertEqual(sink.notifications.count, 2)
    }
}
