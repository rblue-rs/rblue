package com.rblue.example.model

import com.rblue.example.client.CharacteristicIdJson
import com.rblue.example.client.DeviceId
import com.rblue.example.client.Notification
import com.rblue.example.client.PresentedError
import com.rblue.example.client.RblueClient
import com.rblue.example.client.ServiceIdJson
import com.rblue.example.client.ServiceSet
import com.rblue.example.client.toServiceSet
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.rblue_ffi.Rblue
import uniffi.rblue_ffi.RblueException

/**
 * State machine behind one device's detail screen: service discovery, the
 * GATT tree, read/write, the InvalidLength demonstration, and notification
 * subscriptions.
 *
 * Discovery is mandatory on every connection (Android's stack cache can be
 * stale) and re-runs after a reconnect, re-issuing the subscriptions the
 * user kept (CCCD state dies with the connection).
 */
class DeviceDetailModel(
    val device: DeviceId,
    private val client: RblueClient,
) {
    /** One characteristic in the tree; `id` is the canonical
     * `CharacteristicId` JSON for facade calls. */
    data class CharacteristicNode(
        val id: String,
        val serviceUuid: String,
        val uuid: String,
        val properties: Int,
        val descriptorUuids: List<String>,
        val value: ByteArray?,
        val lastAction: String?,
        val subscribed: Boolean,
    ) {
        val canRead: Boolean get() = properties and READ_BIT != 0
        val canWrite: Boolean get() = properties and WRITE_BIT != 0
        val canWriteWithoutResponse: Boolean
            get() = properties and WRITE_WITHOUT_RESPONSE_BIT != 0
        val canNotify: Boolean get() = properties and NOTIFY_BIT != 0
        val canIndicate: Boolean get() = properties and INDICATE_BIT != 0
        val propertyBadges: List<String>
            get() = buildList {
                if (canRead) add("R")
                if (canWrite) add("W")
                if (canWriteWithoutResponse) add("NR")
                if (canNotify) add("N")
                if (canIndicate) add("I")
            }

        override fun equals(other: Any?): Boolean =
            other is CharacteristicNode && other.id == id && other.properties == properties &&
                other.subscribed == subscribed && other.lastAction == lastAction &&
                other.serviceUuid == serviceUuid && other.uuid == uuid &&
                other.descriptorUuids == descriptorUuids && other.value contentEquals value

        override fun hashCode(): Int = id.hashCode()

        companion object {
            private const val READ_BIT = 0x0002
            private const val WRITE_WITHOUT_RESPONSE_BIT = 0x0004
            private const val WRITE_BIT = 0x0008
            private const val NOTIFY_BIT = 0x0010
            private const val INDICATE_BIT = 0x0020
        }
    }

    /** One service in the tree. */
    data class ServiceNode(
        val id: String,
        val uuid: String,
        val primary: Boolean,
        val characteristics: List<CharacteristicNode>,
    )

    private val facade: Rblue? get() = client.facade

    private val _services = MutableStateFlow<List<ServiceNode>>(emptyList())
    val services: StateFlow<List<ServiceNode>> = _services

    private val _rssi = MutableStateFlow<Int?>(null)
    val rssi: StateFlow<Int?> = _rssi

    private val _mtu = MutableStateFlow<Int?>(null)
    val mtu: StateFlow<Int?> = _mtu

    private val _deviceName = MutableStateFlow<String?>(null)
    val deviceName: StateFlow<String?> = _deviceName

    private val _isDiscovering = MutableStateFlow(false)
    val isDiscovering: StateFlow<Boolean> = _isDiscovering

    private val _busyCharacteristicIds = MutableStateFlow<Set<String>>(emptySet())
    val busyCharacteristicIds: StateFlow<Set<String>> = _busyCharacteristicIds

    private val _presentedError = MutableStateFlow<PresentedError?>(null)
    val presentedError: StateFlow<PresentedError?> = _presentedError

    val isConnected: Boolean get() = client.isConnected(device)

    fun notifications(): List<Notification> = client.notifications(device)

    fun dismissError() {
        _presentedError.value = null
    }

    /** Characteristic ids (canonical JSON) the user subscribed; survives
     * rediscovery so subscriptions are re-issued after a reconnect. */
    private val desiredSubscriptions = mutableSetOf<String>()

    /** Serializes GATT operations per model (the facade queues per device,
     * but discovery vs. teardown must not interleave client-side). */
    private val operations = Mutex()

    // MARK: - Discovery

    /** Discovers the full GATT tree (Android has no filtered discovery; the
     * facade filters in software when service UUIDs are given - the demo
     * always discovers everything), reads the connected RSSI and MTU, and
     * re-issues surviving subscriptions. */
    suspend fun discover() = operations.withLock {
        val facade = facade ?: return@withLock
        if (!isConnected || _isDiscovering.value) return@withLock
        _isDiscovering.value = true
        try {
            val deviceJson = device.canonicalJson()
            val tree = facade.discoverServices(deviceJson, emptyList()).toServiceSet()
            _services.value = buildTree(tree)
            for (node in _services.value.flatMap { it.characteristics }) {
                if (node.id in desiredSubscriptions) {
                    facade.subscribe(deviceJson, node.id)
                    mutate(node.id) { it.copy(subscribed = true) }
                }
            }
            if (desiredSubscriptions.isNotEmpty()) {
                client.registerNotificationTarget(device)
            }
            _rssi.value = facade.rssi(deviceJson).toInt()
            // `mtu` is expected to work on Android (MtuQuery capability);
            // degrade silently where a backend lacks it, like iOS does.
            _mtu.value = try {
                facade.mtu(deviceJson).toInt()
            } catch (error: RblueException) {
                null
            }
            // `name` is a non-throwing facade method (null for unknown
            // devices); the scan row already showed the advertised name.
            _deviceName.value = facade.name(deviceJson)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.from(error)
        } finally {
            _isDiscovering.value = false
        }
    }

    /** Builds the service/characteristic tree from a `ServiceSet`,
     * reconstructing the session-scoped `CharacteristicId`s (instance =
     * zero-based index among same-UUID entries in discovery order). */
    private fun buildTree(set: ServiceSet): List<ServiceNode> {
        val serviceInstances = HashMap<String, Int>()
        return set.services.map { service ->
            val serviceInstance = serviceInstances.getOrDefault(service.uuid, 0)
            serviceInstances[service.uuid] = serviceInstance + 1
            val characteristicInstances = HashMap<String, Int>()
            val characteristics = service.characteristics.map { characteristic ->
                val characteristicInstance =
                    characteristicInstances.getOrDefault(characteristic.uuid, 0)
                characteristicInstances[characteristic.uuid] = characteristicInstance + 1
                val identifier = CharacteristicIdJson(
                    service = ServiceIdJson(service.uuid, serviceInstance),
                    uuid = characteristic.uuid,
                    instance = characteristicInstance,
                )
                CharacteristicNode(
                    id = identifier.canonicalJson(),
                    serviceUuid = service.uuid,
                    uuid = characteristic.uuid,
                    properties = characteristic.properties,
                    descriptorUuids = characteristic.descriptors,
                    value = null,
                    lastAction = null,
                    subscribed = false,
                )
            }
            ServiceNode(
                id = "${service.uuid}#$serviceInstance",
                uuid = service.uuid,
                primary = service.primary,
                characteristics = characteristics,
            )
        }
    }

    // MARK: - GATT operations

    /** Reads the characteristic's current value. */
    suspend fun read(node: CharacteristicNode) {
        withBusy(node.id) {
            val facade = facade ?: return@withBusy
            val data = facade.read(device.canonicalJson(), node.id)
            mutate(node.id) {
                it.copy(value = data, lastAction = "读取 ${data.size} 字节")
            }
        }
    }

    /** Writes the payload in the requested mode. `max_write_len` is
     * re-queried per write instead of cached (the MTU is renegotiated per
     * connection and can change mid-session). Oversize payloads are rejected
     * by the facade with the structured `InvalidLength` error. */
    suspend fun write(node: CharacteristicNode, payload: ByteArray, withResponse: Boolean) {
        withBusy(node.id) {
            val facade = facade ?: return@withBusy
            val deviceJson = device.canonicalJson()
            val maxLen = facade.maxWriteLen(deviceJson, node.id, withResponse).toLong()
            facade.write(deviceJson, node.id, payload, withResponse)
            mutate(node.id) {
                it.copy(
                    value = payload,
                    lastAction = "已写入 ${payload.size} 字节 " +
                        "(${if (withResponse) "with response" else "without response"}, " +
                        "max_write_len $maxLen)",
                )
            }
        }
    }

    /** The InvalidLength demonstration: writes a payload one byte over
     * `max_write_len`, expecting the structured rejection. Native Android
     * would silently truncate the write; rblue never truncates, and this
     * makes the platform difference visible. */
    suspend fun writeOversizeDemo(node: CharacteristicNode, withResponse: Boolean) {
        withBusy(node.id) {
            val facade = facade ?: return@withBusy
            val deviceJson = device.canonicalJson()
            val maxLen = facade.maxWriteLen(deviceJson, node.id, withResponse).toLong()
            val payload = ByteArray((maxLen + 1).toInt()) { 0xAB.toByte() }
            try {
                facade.write(deviceJson, node.id, payload, withResponse)
                _presentedError.value = PresentedError(
                    title = "演示异常",
                    detail = "超长载荷(len=${payload.size} > max=$maxLen)没有被拒绝," +
                        "违反统一写语义(InvalidLength),请上报。",
                    structured = "write accepted len=${payload.size} max=$maxLen",
                )
            } catch (error: RblueException) {
                // Expected path: the structured InvalidLength surfaces here.
                _presentedError.value = PresentedError.from(error)
                mutate(node.id) {
                    it.copy(
                        lastAction = "InvalidLength 演示:载荷 ${payload.size} 字节 > max $maxLen",
                    )
                }
            }
        }
    }

    /** Subscribes to value changes / unsubscribes; notification vs.
     * indication follows the characteristic's properties. Values arrive
     * through the client's notification log. */
    suspend fun toggleSubscription(node: CharacteristicNode) {
        withBusy(node.id) {
            val facade = facade ?: return@withBusy
            val deviceJson = device.canonicalJson()
            if (node.id in desiredSubscriptions) {
                facade.unsubscribe(deviceJson, node.id)
                desiredSubscriptions.remove(node.id)
                mutate(node.id) {
                    it.copy(subscribed = false, lastAction = "已取消订阅")
                }
                if (desiredSubscriptions.isEmpty()) {
                    client.unregisterNotificationTarget(device)
                }
            } else {
                facade.subscribe(deviceJson, node.id)
                desiredSubscriptions.add(node.id)
                client.registerNotificationTarget(device)
                mutate(node.id) {
                    it.copy(subscribed = true, lastAction = "已订阅,值将通过通知到达")
                }
            }
        }
    }

    /** Cancels remaining subscriptions when the detail view goes away. A
     * disconnected device needs no work (its CCCD state died with the link);
     * while connected, failures surface as errors. */
    suspend fun unsubscribeAll() {
        val facade = facade ?: return
        if (!isConnected || desiredSubscriptions.isEmpty()) return
        try {
            val deviceJson = device.canonicalJson()
            for (id in desiredSubscriptions.toList()) {
                facade.unsubscribe(deviceJson, id)
            }
            desiredSubscriptions.clear()
            _services.update { list ->
                list.map { service ->
                    service.copy(
                        characteristics = service.characteristics.map { node ->
                            node.copy(subscribed = false)
                        },
                    )
                }
            }
            client.unregisterNotificationTarget(device)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.from(error)
        }
    }

    // MARK: - Platform-limitation probe

    /** Probes `pair()` - expected `Unsupported` (Android pairs implicitly
     * when an encrypted characteristic is accessed; `createBond` is not
     * exposed through the fork). An unexpected success is surfaced as a bug.
     * (`mtu()` is NOT probed on Android: MtuQuery is a supported capability
     * and its value is shown in the header instead.) */
    suspend fun probePair() {
        val facade = facade ?: return
        try {
            facade.pair(device.canonicalJson())
            _presentedError.value = PresentedError(
                title = "演示异常",
                detail = "pair() 成功了;按平台事实 Android 配对是隐式的,应返回 Unsupported,请上报。",
                structured = "pair accepted",
            )
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.from(error)
        }
    }

    // MARK: - Plumbing

    private suspend fun withBusy(characteristicId: String, block: suspend () -> Unit) {
        _busyCharacteristicIds.update { it + characteristicId }
        try {
            block()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.from(error)
        } finally {
            _busyCharacteristicIds.update { it - characteristicId }
        }
    }

    private fun mutate(characteristicId: String, mutate: (CharacteristicNode) -> CharacteristicNode) {
        _services.update { list ->
            list.map { service ->
                service.copy(
                    characteristics = service.characteristics.map { node ->
                        if (node.id == characteristicId) mutate(node) else node
                    },
                )
            }
        }
    }
}

/** Byte-payload rendering helpers for the tree UI. */
object DataRendering {
    fun hex(data: ByteArray): String = data.joinToString(" ") { "%02x".format(it) }

    fun text(data: ByteArray): String = String(data, Charsets.UTF_8)
}
