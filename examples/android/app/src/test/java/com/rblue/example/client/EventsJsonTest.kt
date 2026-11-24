package com.rblue.example.client

import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Decoding tests for the facade's JSON contract (the serde forms of the
 * rblue-api types), plus the ScanFilter wire format the app sends. */
class EventsJsonTest {
    private val fixtureUuid = "4c8853a7-7d4e-4997-9c58-344b9e5e757f"

    @Test
    fun stateChangedDecodes() {
        assertEquals(
            AdapterEvent.StateChanged(AdapterState.POWERED_ON),
            "{\"state_changed\":\"powered_on\"}".toAdapterEvent(),
        )
        assertEquals(
            AdapterEvent.StateChanged(AdapterState.POWERED_OFF),
            "{\"state_changed\":\"powered_off\"}".toAdapterEvent(),
        )
        assertEquals(
            AdapterEvent.StateChanged(AdapterState.UNKNOWN),
            "{\"state_changed\":\"unknown\"}".toAdapterEvent(),
        )
    }

    @Test
    fun deviceDiscoveredDecodesFullAdvertisement() {
        val event = (
            "{\"device_discovered\":{\"advertisement\":{" +
                "\"adv_flags\":null,\"is_connectable\":true,\"local_name\":\"Echo\"," +
                "\"manufacturer_data\":[6,[1,2]],\"rssi\":-42," +
                "\"service_data\":[[\"180f\",[9]]],\"services\":[\"180f\"]," +
                "\"tx_power\":null},\"id\":{\"uuid\":\"$fixtureUuid\"}}}"
            ).toAdapterEvent()
        val discovered = (event as AdapterEvent.DeviceDiscovered).device
        assertEquals(DeviceId.Uuid(fixtureUuid), discovered.id)
        val advertisement = discovered.advertisement
        assertEquals("Echo", advertisement.localName)
        assertEquals(listOf("180f"), advertisement.services)
        assertEquals(-42, advertisement.rssi)
        assertEquals(true, advertisement.isConnectable)
        assertNull(advertisement.advFlags)
        assertNull(advertisement.txPower)
        assertEquals(6, advertisement.manufacturerData?.companyId)
        assertEquals(listOf<Byte>(1, 2), advertisement.manufacturerData?.data)
        assertEquals("180f", advertisement.serviceData.first()?.uuid)
        assertEquals(listOf<Byte>(9), advertisement.serviceData.first()?.data)
    }

    @Test
    fun deviceUpdatedDecodesPair() {
        assertEquals(
            AdapterEvent.DeviceUpdated(DeviceId.Uuid(fixtureUuid), -41),
            "{\"device_updated\":[{\"uuid\":\"$fixtureUuid\"},-41]}".toAdapterEvent(),
        )
    }

    @Test
    fun deviceConnectedDecodes() {
        assertEquals(
            AdapterEvent.DeviceConnected(DeviceId.Uuid(fixtureUuid)),
            "{\"device_connected\":{\"uuid\":\"$fixtureUuid\"}}".toAdapterEvent(),
        )
    }

    @Test
    fun deviceDisconnectedDecodesNullAndPlatformReasons() {
        assertEquals(
            AdapterEvent.DeviceDisconnected(
                DeviceId.Mac(listOf<Byte>(1, 2, 3, 4, 5, 6), AddressType.RANDOM_STATIC),
                null,
            ),
            "{\"device_disconnected\":[{\"mac\":[[1,2,3,4,5,6],\"random_static\"]},null]}"
                .toAdapterEvent(),
        )
        assertEquals(
            AdapterEvent.DeviceDisconnected(DeviceId.Uuid(fixtureUuid), DisconnectReason.Platform(133)),
            "{\"device_disconnected\":[{\"uuid\":\"$fixtureUuid\"},{\"platform\":133}]}"
                .toAdapterEvent(),
        )
    }

    @Test
    fun disconnectReasonStringsDecode() {
        assertEquals(DisconnectReason.Local, "\"local\"".toAdapterEventReason())
        assertEquals(DisconnectReason.TimedOut, "\"timed_out\"".toAdapterEventReason())
        assertEquals(DisconnectReason.Unknown, "\"unknown\"".toAdapterEventReason())
        assertEquals(DisconnectReason.Platform(-7), "{\"platform\":-7}".toAdapterEventReason())
    }

    private fun String.toAdapterEventReason(): DisconnectReason =
        ContractJson.decodeFromString(DisconnectReason.Serializer, this)

    @Test
    fun deviceIdReencodesCanonicalJson() {
        val mac = DeviceId.Mac(listOf<Byte>(1, 2, 3, 4, 5, 6), AddressType.RANDOM_STATIC)
        assertEquals(
            "{\"mac\":[[1,2,3,4,5,6],\"random_static\"]}",
            mac.canonicalJson(),
        )
        assertEquals(
            "{\"uuid\":\"$fixtureUuid\"}",
            DeviceId.Uuid(fixtureUuid).canonicalJson(),
        )
        assertEquals("{\"web\":\"device-42\"}", DeviceId.Web("device-42").canonicalJson())
        // Round trip: decode -> encode is byte-identical.
        assertEquals(
            mac,
            ContractJson.decodeFromString(DeviceId.Serializer, mac.canonicalJson()),
        )
    }

    @Test
    fun deviceIdSummaryFormatsMac() {
        assertEquals(
            "01:02:03:CA:FE:06 (random_static)",
            DeviceId.Mac(
                listOf<Byte>(0x01, 0x02, 0x03, 0xCA.toByte(), 0xFE.toByte(), 0x06),
                AddressType.RANDOM_STATIC,
            ).summary,
        )
    }

    @Test
    fun scanFilterEncodesWireFormat() {
        val encoded = ContractJson.encodeToString(
            ScanFilter.serializer(),
            ScanFilter(namePrefix = "Echo", services = listOf("180F"), rssiThreshold = -80),
        )
        val obj = ContractJson.parseToJsonElement(encoded).jsonObject
        assertEquals(JsonPrimitive("Echo"), obj["name_prefix"])
        assertEquals(buildJsonArray { add(JsonPrimitive("180F")) }, obj["services"])
        assertEquals(JsonPrimitive(-80), obj["rssi_threshold"])
        assertFalse(obj.containsKey("manufacturer_data"))

        // Omitted fields merge over the facade's defaults: an unfiltered
        // request serializes to `{}` (meaning "no filtering").
        val unfiltered = ContractJson.encodeToString(
            ScanFilter.serializer(),
            ScanFilter(),
        )
        assertEquals(buildJsonObject {}, unfiltered.let(ContractJson::parseToJsonElement).jsonObject)
    }

    @Test
    fun scanFilterEncodesManufacturerData() {
        val encoded = ContractJson.encodeToString(
            ScanFilter.serializer(),
            ScanFilter(manufacturerData = ManufacturerFilter(companyId = 0x0059, dataPrefix = listOf(1, 2))),
        )
        val obj = ContractJson.parseToJsonElement(encoded).jsonObject
        assertEquals(
            buildJsonObject {
                put("company_id", 0x0059)
                put("data_prefix", buildJsonArray {
                    add(JsonPrimitive(1))
                    add(JsonPrimitive(2))
                })
            },
            obj["manufacturer_data"],
        )
    }

    @Test
    fun notificationDecodes() {
        val notification = (
            "{\"id\":{\"service\":{\"uuid\":\"180f\",\"instance\":0}," +
                "\"uuid\":\"2a19\",\"instance\":0},\"data\":[104,105]}"
            ).toNotification()
        assertEquals(
            CharacteristicIdJson(
                service = ServiceIdJson("180f", 0),
                uuid = "2a19",
                instance = 0,
            ),
            notification.id,
        )
        assertTrue(notification.data.contentEquals(byteArrayOf(104, 105)))
    }

    @Test
    fun serviceSetDecodesTree() {
        val set = (
            "{\"services\":[" +
                "{\"uuid\":\"180f\",\"primary\":true,\"characteristics\":[" +
                "{\"uuid\":\"2a19\",\"properties\":10,\"descriptors\":[{\"characteristic\":{\"service\":{\"uuid\":\"180f\",\"instance\":0},\"uuid\":\"2a19\",\"instance\":0},\"uuid\":\"2901\",\"instance\":0}]}]}," +
                "{\"uuid\":\"1800\",\"primary\":true,\"characteristics\":[]}]}"
            ).toServiceSet()
        assertEquals(2, set.services.size)
        val characteristic = set.services[0].characteristics[0]
        assertEquals("2a19", characteristic.uuid)
        assertEquals(10, characteristic.properties)
        assertTrue(characteristic.canRead)
        assertTrue(characteristic.canWrite)
        assertFalse(characteristic.canNotify)
        assertEquals(listOf("2901"), characteristic.descriptors.map { it.uuid })
        assertEquals(listOf("R", "W"), characteristic.propertyBadges)
    }

    @Test
    fun capabilitySetParsing() {
        val set = CapabilitySet("ACTIVE_SCAN | GATT_CLIENT")
        assertTrue(set.has("ACTIVE_SCAN"))
        assertTrue(set.has("GATT_CLIENT"))
        assertFalse(set.has("PAIRING"))
        assertTrue(CapabilitySet("0x0").names.isEmpty())
        assertTrue(CapabilitySet("").names.isEmpty())
    }
}
