import XCTest

@testable import RblueExample

/// Dedup/merge/sort semantics of the scan table.
final class ScanListModelTests: XCTestCase {
    private let uuidA = "aaaaaaaa-0000-0000-0000-000000000001"
    private let uuidB = "bbbbbbbb-0000-0000-0000-000000000002"
    private let uuidC = "cccccccc-0000-0000-0000-000000000003"

    private func discovered(
        _ uuid: String, name: String?, rssi: Int16?, services: [String] = []
    ) -> DiscoveredDevice {
        DiscoveredDevice(
            id: .uuid(uuid),
            advertisement: AdvertisementData(
                localName: name,
                services: services,
                serviceData: [],
                manufacturerData: nil,
                txPower: nil,
                advFlags: nil,
                rssi: rssi,
                isConnectable: nil
            )
        )
    }

    func testDiscoveredInsertsAndMergesReSightings() {
        var model = ScanListModel()
        model.apply(discovered: discovered(uuidA, name: "Echo", rssi: -40))
        XCTAssertEqual(model.entries.count, 1)
        // A second discovery of the same identity must not duplicate rows.
        model.apply(discovered: discovered(uuidA, name: nil, rssi: -50))
        XCTAssertEqual(model.entries.count, 1)
        XCTAssertEqual(model.entries[.uuid(uuidA)]?.name, "Echo")
        XCTAssertEqual(model.entries[.uuid(uuidA)]?.rssi, -50)
    }

    func testUpdateMergesIntoExistingRowOnly() {
        var model = ScanListModel()
        model.apply(discovered: discovered(uuidA, name: "Echo", rssi: -40))
        model.apply(updateFor: .uuid(uuidA), rssi: -61)
        XCTAssertEqual(model.entries[.uuid(uuidA)]?.rssi, -61)
        // Updates for never-discovered devices create no rows.
        model.apply(updateFor: .uuid(uuidB), rssi: -30)
        XCTAssertEqual(model.entries.count, 1)
    }

    func testSortedByRssiPutsMissingLastWithStableTies() {
        var model = ScanListModel()
        model.apply(discovered: discovered(uuidA, name: nil, rssi: -50))
        model.apply(discovered: discovered(uuidB, name: nil, rssi: -30))
        model.apply(discovered: discovered(uuidC, name: nil, rssi: nil))
        let order = model.sortedByRssi.map(\.id)
        XCTAssertEqual(order, [.uuid(uuidB), .uuid(uuidA), .uuid(uuidC)])
        // Equal RSSI and both missing: deterministic identity ordering.
        var tie = ScanListModel()
        tie.apply(discovered: discovered(uuidB, name: nil, rssi: nil))
        tie.apply(discovered: discovered(uuidA, name: nil, rssi: nil))
        XCTAssertEqual(tie.sortedByRssi.map(\.id), [.uuid(uuidA), .uuid(uuidB)])
    }

    func testClearDropsEverything() {
        var model = ScanListModel()
        model.apply(discovered: discovered(uuidA, name: nil, rssi: -40))
        model.clear()
        XCTAssertTrue(model.entries.isEmpty)
    }
}
