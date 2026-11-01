package com.rblue.example.client

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

// MARK: - JSON contract
//
// The UniFFI facade carries structured payloads as serde JSON strings using
// the snake_case, externally-tagged representations of the `rblue-api` types
// (see crates/rblue-ffi). These types mirror those representations: event and
// notification payloads received through the EventSink are decoded here, and
// identifiers are re-encoded to the canonical JSON when passed back into
// facade methods.

/** Shared JSON configuration for the facade contract (serde snake_case). */
val ContractJson: Json = Json {
    ignoreUnknownKeys = true
    explicitNulls = false
}

/** Radio/permission state of the adapter (`rblue_api::AdapterState`). */
@Serializable
enum class AdapterState {
    @SerialName("powered_off") POWERED_OFF,
    @SerialName("powered_on") POWERED_ON,
    @SerialName("unauthorized") UNAUTHORIZED,
    @SerialName("unavailable") UNAVAILABLE,
    @SerialName("unknown") UNKNOWN,
}

/** Why a connection dropped (`rblue_api::DisconnectReason`).
 *
 * Serde shape: the plain variants are strings, the `Platform(i32)` tuple
 * variant is the object `{"platform": N}` - hence the custom serializer.
 */
sealed class DisconnectReason {
    data object Local : DisconnectReason()
    data object Remote : DisconnectReason()
    data object TimedOut : DisconnectReason()
    data class Platform(val code: Int) : DisconnectReason()
    data object Unknown : DisconnectReason()

    object Serializer : KSerializer<DisconnectReason> {
        override val descriptor: SerialDescriptor =
            PrimitiveSerialDescriptor("DisconnectReason", PrimitiveKind.STRING)

        override fun deserialize(decoder: Decoder): DisconnectReason {
            return when (val element = decoder.decodeSerializableValue(JsonElement.serializer())) {
                is JsonPrimitive -> when (element.content) {
                    "local" -> Local
                    "remote" -> Remote
                    "timed_out" -> TimedOut
                    "unknown" -> Unknown
                    else ->
                        throw SerializationException("unknown DisconnectReason \"${element.content}\"")
                }
                is JsonObject -> Platform(
                    element["platform"]?.jsonPrimitive?.int
                        ?: throw SerializationException(
                            "platform DisconnectReason requires {\"platform\": N}, got $element",
                        )
                )
                else -> throw SerializationException("invalid DisconnectReason $element")
            }
        }

        override fun serialize(encoder: Encoder, value: DisconnectReason) {
            val element: JsonElement = when (value) {
                is DisconnectReason.Local -> JsonPrimitive("local")
                is DisconnectReason.Remote -> JsonPrimitive("remote")
                is DisconnectReason.TimedOut -> JsonPrimitive("timed_out")
                is DisconnectReason.Unknown -> JsonPrimitive("unknown")
                is DisconnectReason.Platform -> buildJsonObject { put("platform", value.code) }
            }
            encoder.encodeSerializableValue(JsonElement.serializer(), element)
        }
    }
}

/** Address kind carried by a `DeviceId.Mac` (`rblue_api::AddressType`). */
@Serializable
enum class AddressType {
    @SerialName("public") PUBLIC,
    @SerialName("random_static") RANDOM_STATIC,
    @SerialName("resolvable_private") RESOLVABLE_PRIVATE,
    @SerialName("unknown") UNKNOWN,
}

/** Platform-agnostic peripheral identifier (`rblue_api::DeviceId`).
 *
 * Opaque by contract: never constructed from raw values, never persisted.
 * Android only ever reports `Mac`. Decoded values are re-encoded to the
 * canonical JSON when passed back into facade methods; the encoder matches
 * the Rust serde form byte for byte (the facade re-parses either spelling).
 */
sealed class DeviceId {
    data class Mac(val bytes: List<Byte>, val addressType: AddressType) : DeviceId()
    data class Uuid(val value: String) : DeviceId()
    data class Web(val value: String) : DeviceId()

    /** Short human-readable form for list rows and headings. */
    val summary: String
        get() = when (this) {
            is Mac -> bytes.joinToString(":") { "%02X".format(it) } +
                " (${addressType.name.lowercase()})"
            is Uuid -> value
            is Web -> value
        }

    /** Canonical DeviceId JSON for facade method arguments. */
    fun canonicalJson(): String = ContractJson.encodeToString(Serializer, this)

    object Serializer : KSerializer<DeviceId> {
        override val descriptor: SerialDescriptor =
            PrimitiveSerialDescriptor("DeviceId", PrimitiveKind.STRING)

        override fun deserialize(decoder: Decoder): DeviceId {
            val obj = decoder.decodeSerializableValue(JsonElement.serializer()).jsonObject
            return when {
                obj.containsKey("mac") -> {
                    val fields = obj.getValue("mac").jsonArray
                    Mac(
                        bytes = fields[0].jsonArray.map { it.jsonPrimitive.int.toByte() },
                        addressType = ContractJson.decodeFromJsonElement(
                            AddressType.serializer(),
                            fields[1],
                        ),
                    )
                }
                obj.containsKey("uuid") -> Uuid(obj.getValue("uuid").jsonPrimitive.content)
                obj.containsKey("web") -> Web(obj.getValue("web").jsonPrimitive.content)
                else -> throw SerializationException(
                    "no known DeviceId variant key (mac/uuid/web) in $obj",
                )
            }
        }

        override fun serialize(encoder: Encoder, value: DeviceId) {
            val fields: JsonElement = when (value) {
                is Mac -> buildJsonArray {
                    add(buildJsonArray { value.bytes.forEach { add(JsonPrimitive(it.toInt() and 0xFF)) } })
                    add(ContractJson.encodeToJsonElement(AddressType.serializer(), value.addressType))
                }
                is Uuid -> JsonPrimitive(value.value)
                is Web -> JsonPrimitive(value.value)
            }
            val key = when (value) {
                is Mac -> "mac"
                is Uuid -> "uuid"
                is Web -> "web"
            }
            encoder.encodeSerializableValue(
                JsonElement.serializer(),
                buildJsonObject { put(key, fields) },
            )
        }
    }
}

/** `(uuid, bytes)` service-data entry, serialized as a JSON pair. */
@Serializable(with = ServiceDataEntry.Serializer::class)
data class ServiceDataEntry(val uuid: String, val data: List<Byte>) {
    object Serializer : KSerializer<ServiceDataEntry> {
        override val descriptor: SerialDescriptor =
            PrimitiveSerialDescriptor("ServiceDataEntry", PrimitiveKind.STRING)

        override fun deserialize(decoder: Decoder): ServiceDataEntry {
            val fields = decoder.decodeSerializableValue(JsonElement.serializer()).jsonArray
            return ServiceDataEntry(
                uuid = fields[0].jsonPrimitive.content,
                data = fields[1].jsonArray.map { it.jsonPrimitive.int.toByte() },
            )
        }

        override fun serialize(encoder: Encoder, value: ServiceDataEntry) {
            val fields = buildJsonArray {
                add(JsonPrimitive(value.uuid))
                add(buildJsonArray { value.data.forEach { add(JsonPrimitive(it.toInt() and 0xFF)) } })
            }
            encoder.encodeSerializableValue(JsonElement.serializer(), fields)
        }
    }
}

/** `(company_id, bytes)` manufacturer-data entry, serialized as a JSON pair. */
@Serializable(with = ManufacturerData.Serializer::class)
data class ManufacturerData(val companyId: Int, val data: List<Byte>) {
    object Serializer : KSerializer<ManufacturerData> {
        override val descriptor: SerialDescriptor =
            PrimitiveSerialDescriptor("ManufacturerData", PrimitiveKind.STRING)

        override fun deserialize(decoder: Decoder): ManufacturerData {
            val fields = decoder.decodeSerializableValue(JsonElement.serializer()).jsonArray
            return ManufacturerData(
                companyId = fields[0].jsonPrimitive.int,
                data = fields[1].jsonArray.map { it.jsonPrimitive.int.toByte() },
            )
        }

        override fun serialize(encoder: Encoder, value: ManufacturerData) {
            val fields = buildJsonArray {
                add(JsonPrimitive(value.companyId))
                add(buildJsonArray { value.data.forEach { add(JsonPrimitive(it.toInt() and 0xFF)) } })
            }
            encoder.encodeSerializableValue(JsonElement.serializer(), fields)
        }
    }
}

/** Parsed advertisement (`rblue_api::AdvertisementData`). */
@Serializable
data class AdvertisementData(
    @SerialName("local_name") val localName: String? = null,
    val services: List<String> = emptyList(),
    @SerialName("service_data") val serviceData: List<ServiceDataEntry> = emptyList(),
    @SerialName("manufacturer_data") val manufacturerData: ManufacturerData? = null,
    @SerialName("tx_power") val txPower: Int? = null,
    @SerialName("adv_flags") val advFlags: Int? = null,
    val rssi: Int? = null,
    @SerialName("is_connectable") val isConnectable: Boolean? = null,
)

/** One device sighting (`rblue_api::DiscoveredDevice`). */
@Serializable
data class DiscoveredDevice(
    @Serializable(with = DeviceId.Serializer::class) val id: DeviceId,
    val advertisement: AdvertisementData = AdvertisementData(),
)

/** Streamed adapter events (`rblue_api::AdapterEvent`).
 *
 * Externally tagged: `{"device_updated": [<DeviceId>, <rssi>]}` and similar;
 * single-field tuple variants serialize their payload directly.
 */
sealed class AdapterEvent {
    data class StateChanged(val state: AdapterState) : AdapterEvent()
    data class DeviceDiscovered(val device: DiscoveredDevice) : AdapterEvent()
    data class DeviceUpdated(val id: DeviceId, val rssi: Int) : AdapterEvent()
    data class DeviceConnected(val id: DeviceId) : AdapterEvent()
    data class DeviceDisconnected(val id: DeviceId, val reason: DisconnectReason?) : AdapterEvent()

    object Serializer : KSerializer<AdapterEvent> {
        override val descriptor: SerialDescriptor =
            PrimitiveSerialDescriptor("AdapterEvent", PrimitiveKind.STRING)

        override fun deserialize(decoder: Decoder): AdapterEvent {
            val obj = decoder.decodeSerializableValue(JsonElement.serializer()).jsonObject
            if (obj.size != 1) {
                throw SerializationException(
                    "expected exactly one externally tagged AdapterEvent key, got ${obj.keys}",
                )
            }
            val (key, payload) = obj.entries.first()
            return when (key) {
                "state_changed" -> StateChanged(
                    ContractJson.decodeFromJsonElement(AdapterState.serializer(), payload)
                )
                "device_discovered" -> DeviceDiscovered(
                    ContractJson.decodeFromJsonElement(DiscoveredDevice.serializer(), payload)
                )
                "device_updated" -> {
                    val fields = payload.jsonArray
                    DeviceUpdated(
                        id = ContractJson.decodeFromJsonElement(DeviceId.Serializer, fields[0]),
                        rssi = fields[1].jsonPrimitive.int,
                    )
                }
                "device_connected" -> DeviceConnected(
                    ContractJson.decodeFromJsonElement(DeviceId.Serializer, payload)
                )
                "device_disconnected" -> {
                    val fields = payload.jsonArray
                    DeviceDisconnected(
                        id = ContractJson.decodeFromJsonElement(DeviceId.Serializer, fields[0]),
                        reason = when (fields[1]) {
                            is JsonNull -> null
                            else ->
                                ContractJson.decodeFromJsonElement(DisconnectReason.Serializer, fields[1])
                        },
                    )
                }
                else -> throw SerializationException("unknown AdapterEvent variant \"$key\"")
            }
        }

        override fun serialize(encoder: Encoder, value: AdapterEvent) {
            val key: String
            val payload: JsonElement = when (value) {
                is StateChanged -> {
                    key = "state_changed"
                    ContractJson.encodeToJsonElement(AdapterState.serializer(), value.state)
                }
                is DeviceDiscovered -> {
                    key = "device_discovered"
                    ContractJson.encodeToJsonElement(DiscoveredDevice.serializer(), value.device)
                }
                is DeviceUpdated -> {
                    key = "device_updated"
                    buildJsonArray {
                        add(ContractJson.encodeToJsonElement(DeviceId.Serializer, value.id))
                        add(JsonPrimitive(value.rssi))
                    }
                }
                is DeviceConnected -> {
                    key = "device_connected"
                    ContractJson.encodeToJsonElement(DeviceId.Serializer, value.id)
                }
                is DeviceDisconnected -> {
                    key = "device_disconnected"
                    buildJsonArray {
                        add(ContractJson.encodeToJsonElement(DeviceId.Serializer, value.id))
                        add(
                            value.reason?.let {
                                ContractJson.encodeToJsonElement(DisconnectReason.Serializer, it)
                            } ?: JsonNull,
                        )
                    }
                }
            }
            encoder.encodeSerializableValue(
                JsonElement.serializer(),
                buildJsonObject { put(key, payload) },
            )
        }
    }
}

/** The discovered GATT tree (`rblue_api::ServiceSet`). */
@Serializable
data class ServiceSet(val services: List<ServiceJson> = emptyList())

/** One GATT service (`rblue_api::Service`). */
@Serializable
data class ServiceJson(
    val uuid: String,
    val primary: Boolean,
    val characteristics: List<CharacteristicJson> = emptyList(),
)

/** One GATT characteristic (`rblue_api::Characteristic`).
 *
 * `properties` is the raw `CharacteristicProperties` bitflags value.
 */
@Serializable
data class CharacteristicJson(
    val uuid: String,
    val properties: Int = 0,
    val descriptors: List<String> = emptyList(),
) {
    // GATT property bits (Bluetooth Core Spec, Vol 3 Part G 3.3.1.1).
    val canRead: Boolean get() = properties and READ_BIT != 0
    val canWrite: Boolean get() = properties and WRITE_BIT != 0
    val canWriteWithoutResponse: Boolean get() = properties and WRITE_WITHOUT_RESPONSE_BIT != 0
    val canNotify: Boolean get() = properties and NOTIFY_BIT != 0
    val canIndicate: Boolean get() = properties and INDICATE_BIT != 0

    /** Compact property badges for the tree UI, e.g. `R W NR N`. */
    val propertyBadges: List<String>
        get() = buildList {
            if (canRead) add("R")
            if (canWrite) add("W")
            if (canWriteWithoutResponse) add("NR")
            if (canNotify) add("N")
            if (canIndicate) add("I")
        }

    companion object {
        const val READ_BIT = 0x0002
        const val WRITE_WITHOUT_RESPONSE_BIT = 0x0004
        const val WRITE_BIT = 0x0008
        const val NOTIFY_BIT = 0x0010
        const val INDICATE_BIT = 0x0020
    }
}

/** Identity of a service within a connection (`rblue_api::ServiceId`). */
@Serializable
data class ServiceIdJson(
    val uuid: String,
    val instance: Int,
)

/** Identity of a characteristic within one connected session
 * (`rblue_api::CharacteristicId`).
 *
 * Session-scoped by contract: valid only for the current connection, never
 * persisted; after a reconnect fresh identifiers come from a new
 * `discover_services` call.
 */
@Serializable
data class CharacteristicIdJson(
    val service: ServiceIdJson,
    val uuid: String,
    val instance: Int,
) {
    /** Canonical CharacteristicId JSON for facade method arguments. */
    fun canonicalJson(): String = ContractJson.encodeToString(serializer(), this)
}

/** One GATT notification payload (`rblue_api::Notification`). */
@Serializable
data class Notification(
    @SerialName("id") val id: CharacteristicIdJson,
    @Serializable(with = ByteArrayAsListSerializer::class) val data: ByteArray,
) {
    override fun equals(other: Any?): Boolean =
        other is Notification && id == other.id && data.contentEquals(other.data)

    override fun hashCode(): Int = id.hashCode() * 31 + data.contentHashCode()
}

/** `ByteArray` <-> JSON array of unsigned byte numbers (the serde `Vec<u8>`
 * wire form). */
object ByteArrayAsListSerializer : KSerializer<ByteArray> {
    private val delegate = ListSerializer(Int.serializer())

    override val descriptor: SerialDescriptor = delegate.descriptor

    override fun deserialize(decoder: Decoder): ByteArray =
        decoder.decodeSerializableValue(delegate).map { it.toByte() }.toByteArray()

    override fun serialize(encoder: Encoder, value: ByteArray) =
        encoder.encodeSerializableValue(delegate, value.map { it.toInt() and 0xFF })
}

/** Scan filter request (`rblue_api::ScanFilter`).
 *
 * Null/empty fields fall back to the facade's defaults through its
 * `parse_with_defaults` merge, so omitted keys mean "no filtering".
 */
@Serializable
data class ScanFilter(
    @SerialName("name_prefix") val namePrefix: String? = null,
    val services: List<String> = emptyList(),
    @SerialName("manufacturer_data") val manufacturerData: ManufacturerFilter? = null,
    @SerialName("rssi_threshold") val rssiThreshold: Int? = null,
)

/** Manufacturer-data filter (`rblue_api::ManufacturerFilter`). */
@Serializable
data class ManufacturerFilter(
    @SerialName("company_id") val companyId: Int,
    @SerialName("data_prefix") val dataPrefix: List<Int> = emptyList(),
)

/** The adapter's capability set, parsed from the facade's `|`-separated
 * bitflags rendering (an empty set renders as `0x0`). */
class CapabilitySet(raw: String) {
    val names: Set<String> = raw.split("|")
        .map { it.trim() }
        .filterTo(mutableSetOf()) { it in KNOWN_NAMES }

    fun has(name: String): Boolean = name in names

    companion object {
        val KNOWN_NAMES: Set<String> = setOf(
            "PASSIVE_SCAN",
            "ACTIVE_SCAN",
            "SCAN_RESPONSE",
            "GATT_CLIENT",
            "GATT_SERVER",
            "ADVERTISE",
            "RSSI_DURING_CONNECTION",
            "MTU_NEGOTIATION",
            "MTU_QUERY",
            "PAIRING",
            "WATCH_ADVERTISEMENTS",
            "REQUEST_DEVICE_PICKER",
            "BACKGROUND_SCAN",
        )
    }
}

// MARK: - Decode/encode entry points

/** Decodes one facade event JSON (`AdapterEvent` wire form). */
fun String.toAdapterEvent(): AdapterEvent =
    ContractJson.decodeFromString(AdapterEvent.Serializer, this)

/** Decodes one facade notification JSON (`Notification` wire form). */
fun String.toNotification(): Notification =
    ContractJson.decodeFromString(Notification.serializer(), this)

/** Decodes a `ServiceSet` JSON (the `discover_services` result). */
fun String.toServiceSet(): ServiceSet =
    ContractJson.decodeFromString(ServiceSet.serializer(), this)
