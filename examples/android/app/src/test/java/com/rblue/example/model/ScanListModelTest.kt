package com.rblue.example.model

import com.rblue.example.client.AddressType
import com.rblue.example.client.AdvertisementData
import com.rblue.example.client.DeviceId
import com.rblue.example.client.DiscoveredDevice
import com.rblue.example.client.ManufacturerData
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Scan table semantics: dedupe by DeviceId, merge of re-sightings, RSSI
 * sort with deterministic tie-breaks (pure model, no hardware). */
class ScanListModelTest {
    private fun mac(vararg bytes: Int) = DeviceId.Mac(
        bytes.map { it.toByte() },
        AddressType.RANDOM_STATIC,
    )

    private fun discovered(
        id: DeviceId,
        name: String? = null,
        rssi: Int? = null,
        services: List<String> = emptyList(),
        companyId: Int? = null,
    ) = DiscoveredDevice(
        id = id,
        advertisement = AdvertisementData(
            localName = name,
            services = services,
            manufacturerData = companyId?.let { ManufacturerData(it, emptyList()) },
            rssi = rssi,
        ),
    )

    @Test
    fun duplicatesDedupeByIdentity() {
        val id = mac(1, 2, 3, 4, 5, 6)
        val model = ScanListModel()
            .apply(discovered(id, name = "Echo", rssi = -40))
            .apply(discovered(id, name = "Echo", rssi = -42))
        assertEquals(1, model.entries.size)
        assertEquals(-42, model.entries[id]?.rssi)
    }

    @Test
    fun mergeKeepsOlderFieldsWhenNewSightingOmitsThem() {
        val id = mac(1, 2, 3, 4, 5, 6)
        val model = ScanListModel()
            .apply(discovered(id, name = "Echo", rssi = -40, services = listOf("180f"), companyId = 6))
            .apply(discovered(id, rssi = -55))
        val entry = model.entries[id]!!
        assertEquals("Echo", entry.name)
        assertEquals(-55, entry.rssi)
        assertEquals(listOf("180f"), entry.services)
        assertEquals(6, entry.manufacturerCompanyId)
    }

    @Test
    fun distinctAddressesAreDistinctDevices() {
        val model = ScanListModel()
            .apply(discovered(mac(1, 2, 3, 4, 5, 6), name = "A", rssi = -40))
            .apply(discovered(mac(6, 5, 4, 3, 2, 1), name = "B", rssi = -60))
        assertEquals(2, model.entries.size)
    }

    @Test
    fun updateForKnownDeviceRefreshesRssi() {
        val id = mac(1, 2, 3, 4, 5, 6)
        val model = ScanListModel()
            .apply(discovered(id, name = "Echo", rssi = -40))
            .applyUpdate(id, -71)
        assertEquals(-71, model.entries[id]?.rssi)
    }

    @Test
    fun updateForUnknownDeviceIsIgnored() {
        val id = mac(1, 2, 3, 4, 5, 6)
        val updated = ScanListModel().applyUpdate(id, -71)
        assertNull(updated.entries[id])
    }

    @Test
    fun sortedByRssiDescendingMissingLastTiesBySummary() {
        val a = mac(0x0A, 0, 0, 0, 0, 1)
        val b = mac(0x0B, 0, 0, 0, 0, 1)
        val c = mac(0x0C, 0, 0, 0, 0, 1)
        val d = mac(0x0D, 0, 0, 0, 0, 1)
        val model = ScanListModel()
            .apply(discovered(a, name = "A", rssi = -40))
            .apply(discovered(b, name = "B", rssi = -40))
            .apply(discovered(c, name = "C", rssi = -90))
            .apply(discovered(d, name = "D"))
        assertEquals(
            listOf("A", "B", "C", "D"),
            model.sortedByRssi.map { it.name },
        )
    }

    @Test
    fun clearDropsEveryRow() {
        val id = mac(1, 2, 3, 4, 5, 6)
        val model = ScanListModel().apply(discovered(id, name = "Echo"))
        assertEquals(0, model.clear().entries.size)
    }
}
