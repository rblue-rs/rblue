import Foundation
import XCTest

@testable import RblueExample

/// Decoding tests for the facade's JSON contract (the serde forms of the
/// rblue-api types), plus the ScanFilter wire format the app sends.
final class EventsJsonTests: XCTestCase {
    private let fixtureUuid = "4c8853a7-7d4e-4997-9c58-344b9e5e757f"

    private func decodeEvent(_ json: String) throws -> AdapterEvent {
        try JSONDecoder().decode(AdapterEvent.self, from: Data(json.utf8))
    }

    func testStateChangedDecodes() throws {
        let event = try decodeEvent(#"{"state_changed":"powered_on"}"#)
        XCTAssertEqual(event, .stateChanged(.poweredOn))
    }

    func testDeviceDiscoveredDecodesFullAdvertisement() throws {
        let event = try decodeEvent(
            #"{"device_discovered":{"advertisement":{"adv_flags":null,"is_connectable":true,"local_name":"Echo","manufacturer_data":[6,[1,2]],"rssi":-42,"service_data":[["180f",[9]]],"services":["180f"],"tx_power":null},"id":{"uuid":"\#(fixtureUuid)"}}}"#
        )
        guard case .deviceDiscovered(let device) = event else {
            return XCTFail("expected device_discovered, got \(event)")
        }
        XCTAssertEqual(device.id, .uuid(fixtureUuid))
        XCTAssertEqual(device.advertisement.localName, "Echo")
        XCTAssertEqual(device.advertisement.services, ["180f"])
        XCTAssertEqual(device.advertisement.rssi, -42)
        XCTAssertEqual(device.advertisement.isConnectable, true)
        XCTAssertEqual(device.advertisement.manufacturerData?.companyId, 6)
        XCTAssertEqual(device.advertisement.manufacturerData?.data, [1, 2])
        XCTAssertEqual(device.advertisement.serviceData.first?.uuid, "180f")
        XCTAssertEqual(device.advertisement.serviceData.first?.data, [9])
    }

    func testDeviceUpdatedDecodesPair() throws {
        let event = try decodeEvent(
            #"{"device_updated":[{"uuid":"\#(fixtureUuid)"},-41]}"#
        )
        XCTAssertEqual(event, .deviceUpdated(.uuid(fixtureUuid), rssi: -41))
    }

    func testDeviceConnectedDecodes() throws {
        let event = try decodeEvent(#"{"device_connected":{"uuid":"\#(fixtureUuid)"}}"#)
        XCTAssertEqual(event, .deviceConnected(.uuid(fixtureUuid)))
    }

    func testDeviceDisconnectedDecodesNullAndPlatformReasons() throws {
        let unknown = try decodeEvent(
            #"{"device_disconnected":[{"mac":[[1,2,3,4,5,6],"random_static"]},null]}"#
        )
        XCTAssertEqual(
            unknown,
            .deviceDisconnected(.mac([1, 2, 3, 4, 5, 6], .randomStatic), nil)
        )
        let platform = try decodeEvent(
            #"{"device_disconnected":[{"uuid":"\#(fixtureUuid)"},{"platform":133}]}"#
        )
        XCTAssertEqual(
            platform,
            .deviceDisconnected(.uuid(fixtureUuid), .platform(133))
        )
    }

    func testDisconnectReasonStringsDecode() throws {
        let decoder = JSONDecoder()
        XCTAssertEqual(
            try decoder.decode(DisconnectReason.self, from: Data(#""local""#.utf8)),
            .local
        )
        XCTAssertEqual(
            try decoder.decode(DisconnectReason.self, from: Data(#""timed_out""#.utf8)),
            .timedOut
        )
        XCTAssertEqual(
            try decoder.decode(DisconnectReason.self, from: Data(#""unknown""#.utf8)),
            .unknown
        )
        XCTAssertEqual(
            try decoder.decode(DisconnectReason.self, from: Data(#"{"platform":-7}"#.utf8)),
            .platform(-7)
        )
    }

    func testDeviceIdReencodesCanonicalJson() throws {
        let decoder = JSONDecoder()
        let mac: DeviceId = try decoder.decode(
            DeviceId.self, from: Data(#"{"mac":[[1,2,3,4,5,6],"random_static"]}"#.utf8))
        XCTAssertEqual(
            try mac.deviceJson(),
            #"{"mac":[[1,2,3,4,5,6],"random_static"]}"#
        )
        let web: DeviceId = try decoder.decode(
            DeviceId.self, from: Data(#"{"web":"device-42"}"#.utf8))
        XCTAssertEqual(try web.deviceJson(), #"{"web":"device-42"}"#)
        XCTAssertEqual(try DeviceId.uuid(fixtureUuid).deviceJson(), "{\"uuid\":\"\(fixtureUuid)\"}")
    }

    func testScanFilterEncodesWireFormat() throws {
        let filter = ScanFilter(namePrefix: "Echo", services: ["180F"], rssiThreshold: -80)
        XCTAssertEqual(
            try TestJSON.object(try filter.jsonEncoded()),
            ["name_prefix": "Echo", "services": ["180F"], "rssi_threshold": -80]
                as NSDictionary
        )
        // Optional fields are skipped by the synthesized encoder; `services`
        // always serializes (an empty array merges over the facade's default
        // empty array, so the meaning is identical).
        let unfiltered = ScanFilter(namePrefix: nil, services: [], rssiThreshold: nil)
        XCTAssertEqual(
            try TestJSON.object(try unfiltered.jsonEncoded()),
            ["services": [String]()] as NSDictionary
        )
    }

    func testNotificationDecodes() throws {
        let notification = try JSONDecoder().decode(
            Notification.self,
            from: Data(
                #"{"id":{"service":{"uuid":"180f","instance":0},"uuid":"2a19","instance":0},"data":[104,105]}"#
                    .utf8
            )
        )
        XCTAssertEqual(
            notification.id,
            CharacteristicId(
                service: ServiceIdJson(uuid: "180f", instance: 0), uuid: "2a19", instance: 0)
        )
        XCTAssertEqual(notification.data, [104, 105])
    }

    func testServiceSetDecodesTree() throws {
        let set = try JSONDecoder().decode(
            ServiceSet.self,
            from: Data(
                #"{"services":[{"uuid":"180f","primary":true,"characteristics":[{"uuid":"2a19","properties":10,"descriptors":["2901"]}]},{"uuid":"1800","primary":true,"characteristics":[]}]}"#
                    .utf8
            )
        )
        XCTAssertEqual(set.services.count, 2)
        let characteristic = set.services[0].characteristics[0]
        XCTAssertEqual(characteristic.uuid, "2a19")
        XCTAssertEqual(characteristic.properties, 10)
        XCTAssertTrue(characteristic.canRead)
        XCTAssertTrue(characteristic.canWrite)
        XCTAssertFalse(characteristic.canNotify)
        XCTAssertEqual(characteristic.descriptors, ["2901"])
        XCTAssertEqual(set.services[0].characteristics[0].propertyBadges, ["R", "W"])
    }

    func testCapabilitySetParsing() {
        let set = CapabilitySet("ACTIVE_SCAN | GATT_CLIENT")
        XCTAssertTrue(set.has("ACTIVE_SCAN"))
        XCTAssertTrue(set.has("GATT_CLIENT"))
        XCTAssertFalse(set.has("PAIRING"))
        XCTAssertTrue(CapabilitySet("0x0").names.isEmpty)
        XCTAssertTrue(CapabilitySet("").names.isEmpty)
    }
}
