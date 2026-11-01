package com.rblue.example.model

import com.rblue.example.client.AdapterEvent
import com.rblue.example.client.DeviceId
import com.rblue.example.client.DiscoveredDevice

/** One scan row: the merged view of every sighting of one device. */
data class ScanEntry(
    val id: DeviceId,
    val name: String?,
    val rssi: Int?,
    val services: List<String>,
    val manufacturerCompanyId: Int?,
    val isConnectable: Boolean?,
)

/**
 * Scan result table: dedupes by `DeviceId`, merges re-sightings into the
 * existing row, and sorts by RSSI.
 *
 * Pure immutable value type so the dedup/update semantics are unit-testable
 * without hardware; the client holds it in a `StateFlow`.
 */
data class ScanListModel(val entries: Map<DeviceId, ScanEntry> = emptyMap()) {

    /** Rows sorted by RSSI descending (missing RSSI last, ties broken by
     * identity summary for a deterministic order). */
    val sortedByRssi: List<ScanEntry>
        get() = entries.values.sortedWith(
            compareBy(
                { it.rssi == null },
                { it.rssi?.let { rssi -> -rssi } ?: 0 },
                { it.id.summary },
            ),
        )

    /** Inserts a discovered device, merging with any previous sighting
     * (defensive: the backend publishes re-sightings as updates, not as a
     * second discovery, but the merge keeps the model total). */
    fun apply(discovered: DiscoveredDevice): ScanListModel {
        val advertisement = discovered.advertisement
        val incoming = ScanEntry(
            id = discovered.id,
            name = advertisement.localName,
            rssi = advertisement.rssi,
            services = advertisement.services,
            manufacturerCompanyId = advertisement.manufacturerData?.companyId,
            isConnectable = advertisement.isConnectable,
        )
        val merged = entries[discovered.id]?.let { existing ->
            incoming.copy(
                name = incoming.name ?: existing.name,
                rssi = incoming.rssi ?: existing.rssi,
                isConnectable = incoming.isConnectable ?: existing.isConnectable,
                manufacturerCompanyId =
                    incoming.manufacturerCompanyId ?: existing.manufacturerCompanyId,
                services = incoming.services.ifEmpty { existing.services },
            )
        } ?: incoming
        return copy(entries = entries + (discovered.id to merged))
    }

    /** Applies a re-advertisement: RSSI updated for a known device. Unknown
     * devices are ignored (the model only lists what it saw discovered). */
    fun applyUpdate(id: DeviceId, rssi: Int): ScanListModel {
        val existing = entries[id] ?: return this
        return copy(entries = entries + (id to existing.copy(rssi = rssi)))
    }

    /** Drops every row; used when a scan restarts. */
    fun clear(): ScanListModel = ScanListModel()

    companion object {
        /** Convenience intake used by the client's event loop. */
        fun applying(event: AdapterEvent, model: ScanListModel): ScanListModel = when (event) {
            is AdapterEvent.DeviceDiscovered -> model.apply(event.device)
            is AdapterEvent.DeviceUpdated -> model.applyUpdate(event.id, event.rssi)
            else -> model
        }
    }
}
