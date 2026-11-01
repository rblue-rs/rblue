import Foundation

/// Scan result table: dedupes by `DeviceId`, merges re-sightings into the
/// existing row, and sorts by RSSI.
///
/// Pure value type so the dedup/update semantics are unit-testable without
/// hardware.
struct ScanListModel: Equatable, Sendable {
    struct Entry: Equatable, Identifiable, Sendable {
        let id: DeviceId
        var name: String?
        var rssi: Int16?
        var services: [String]
        var manufacturerCompanyId: UInt16?
        var isConnectable: Bool?
    }

    private(set) var entries: [DeviceId: Entry] = [:]

    /// Rows sorted by RSSI descending (missing RSSI last, ties broken by
    /// identity for a deterministic order).
    var sortedByRssi: [Entry] {
        entries.values.sorted { lhs, rhs in
            switch (lhs.rssi, rhs.rssi) {
            case let (l?, r?):
                return l == r ? lhs.id.summary < rhs.id.summary : l > r
            case (_?, nil):
                return true
            case (nil, _?):
                return false
            case (nil, nil):
                return lhs.id.summary < rhs.id.summary
            }
        }
    }

    /// Inserts a discovered device, merging with any previous sighting
    /// (defensive: the backend publishes re-sightings as updates, not as a
    /// second discovery, but the merge keeps the model total).
    mutating func apply(discovered: DiscoveredDevice) {
        let advertisement = discovered.advertisement
        var entry = Entry(
            id: discovered.id,
            name: advertisement.localName,
            rssi: advertisement.rssi,
            services: advertisement.services,
            manufacturerCompanyId: advertisement.manufacturerData?.companyId,
            isConnectable: advertisement.isConnectable
        )
        if let existing = entries[discovered.id] {
            entry.name = entry.name ?? existing.name
            entry.rssi = entry.rssi ?? existing.rssi
            entry.isConnectable = entry.isConnectable ?? existing.isConnectable
            entry.manufacturerCompanyId =
                entry.manufacturerCompanyId ?? existing.manufacturerCompanyId
            entry.services = entry.services.isEmpty ? existing.services : entry.services
        }
        entries[discovered.id] = entry
    }

    /// Applies a re-advertisement: RSSI (and payload) updated for a known
    /// device. Unknown devices are ignored (the model only lists what it saw
    /// discovered).
    mutating func apply(updateFor id: DeviceId, rssi: Int16) {
        entries[id]?.rssi = rssi
    }

    /// Drops every row; used when a scan restarts.
    mutating func clear() {
        entries = [:]
    }
}
