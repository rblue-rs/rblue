package com.rblue.example.client

import com.rblue.example.model.ScanListModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import uniffi.rblue_ffi.Rblue

/**
 * The app's single facade owner: installs the `EventSink` (through
 * [EventPump]'s `callbackFlow`), converts the raw JSON event streams into
 * typed state ([ScanListModel], connection bookkeeping, notification logs),
 * and runs the Kotlin-side automatic reconnect. The facade does not export a
 * supervisor, so the reconnect policy lives here - same as the iOS example.
 *
 * The facade is constructed once (constructor injection for tests); all
 * mutable state is exposed as `StateFlow`s, mutated only from the client's
 * own `Dispatchers.Main` scope.
 */
class RblueClient(
    facadeFactory: () -> Rblue = { Rblue() },
    private val reconnectBaseMillis: Long = ReconnectPolicy.DEFAULT_BASE_MILLIS,
) {
    /** Per-device reconnect bookkeeping shown in the UI banner. */
    data class ReconnectState(
        val attempt: Int,
        val nextDelayMillis: Long,
        /** Structured rendering of the last failed attempt, if any. */
        val lastError: String?,
    )

    /** Connection bookkeeping per device. */
    data class ConnectionState(val connected: Boolean, val reconnect: ReconnectState?)

    /** Upper bound of the per-device notification log (demo UI only). */
    val notificationLogLimit = 100

    // Diagnostics captured from the facade at construction time (sync calls).
    val backendVersion: String
    val adapterId: String
    val capabilities: CapabilitySet

    /** The facade when one is available; `null` when construction failed
     * (the error is presented; the UI degrades, mirroring the iOS example's
     * no-facade path). */
    val facade: Rblue?

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var pumpJob: Job? = null
    private var scanRunning = false
    private val reconnectJobs = mutableMapOf<DeviceId, Job>()

    /** Devices the user connected through this client and has not explicitly
     * disconnected; these auto-reconnect on unexpected disconnects. */
    private val autoReconnect = mutableSetOf<DeviceId>()

    /** Devices with at least one active subscription, most recent first.
     * The facade's `Notification` JSON carries no device identity, so
     * attribution uses this registry (see KNOWN-GAPS.md). */
    private val subscribedDevices = mutableListOf<DeviceId>()

    private val _adapterState = MutableStateFlow(AdapterState.UNKNOWN)
    val adapterState: StateFlow<AdapterState> = _adapterState

    private val _scan = MutableStateFlow(ScanListModel())
    val scan: StateFlow<ScanListModel> = _scan

    private val _connections = MutableStateFlow<Map<DeviceId, ConnectionState>>(emptyMap())
    val connections: StateFlow<Map<DeviceId, ConnectionState>> = _connections

    private val _isScanning = MutableStateFlow(false)
    val isScanning: StateFlow<Boolean> = _isScanning

    /** Newest-last per-device notification log, capped at
     * [notificationLogLimit]. */
    private val _notificationLogs =
        MutableStateFlow<Map<DeviceId, List<Notification>>>(emptyMap())
    val notificationLogs: StateFlow<Map<DeviceId, List<Notification>>> = _notificationLogs

    private val _presentedError = MutableStateFlow<PresentedError?>(null)
    val presentedError: StateFlow<PresentedError?> = _presentedError

    init {
        var created: Rblue? = null
        var version = "unavailable"
        var id = "unavailable"
        var caps = CapabilitySet("0x0")
        try {
            val facade = facadeFactory()
            created = facade
            version = facade.backendVersion()
            id = facade.adapterId()
            caps = CapabilitySet(facade.capabilities())
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.from(error)
        }
        facade = created
        backendVersion = version
        adapterId = id
        capabilities = caps
    }

    fun isConnected(id: DeviceId): Boolean = _connections.value[id]?.connected ?: false

    fun reconnectState(id: DeviceId): ReconnectState? = _connections.value[id]?.reconnect

    fun notifications(id: DeviceId): List<Notification> = _notificationLogs.value[id] ?: emptyList()

    /** Runs `block` on the client scope (used for view teardown paths such
     * as unsubscribe-all when the detail screen goes away). */
    fun launchInBackground(block: suspend () -> Unit) {
        scope.launch { block() }
    }

    // MARK: - Startup

    /** Installs the event sink and starts consuming the multiplexed stream.
     * Must run before [initializeAdapter] so state events are not missed.
     * Idempotent. */
    fun start() {
        val facade = facade ?: return
        if (pumpJob != null) return
        pumpJob = scope.launch {
            EventPump(facade).messages.collect { message ->
                when (message) {
                    is PumpMessage.Event -> handleEventJson(message.json)
                    is PumpMessage.Notification -> handleNotificationJson(message.json)
                }
            }
        }
    }

    /** Initializes the adapter. On Android this is a light setup step: the
     * fork tracks no radio state, so even a powered-off radio succeeds here -
     * the first real operation reports the radio condition (see the platform
     * notes page). */
    suspend fun initializeAdapter() {
        presentErrors { facade?.initialize() }
    }

    // MARK: - Scan

    /** Starts a scan with the optional filters; rows are cleared so stale
     * devices from a previous filter do not linger. */
    suspend fun startScan(namePrefix: String?, serviceUuid: String?, rssiThreshold: Int?) {
        val facade = facade ?: return
        if (scanRunning) return
        val filter = ScanFilter(
            namePrefix = namePrefix?.trim()?.takeIf { it.isNotEmpty() },
            services = serviceUuid?.trim()?.takeIf { it.isNotEmpty() }?.let { listOf(it) }
                ?: emptyList(),
            rssiThreshold = rssiThreshold,
        )
        presentErrors {
            facade.startScan(ContractJson.encodeToString(ScanFilter.serializer(), filter))
            scanRunning = true
            _isScanning.value = true
            _scan.value = _scan.value.clear()
        }
    }

    /** Stops the running scan. Idempotent. */
    suspend fun stopScan() {
        presentErrors { facade?.stopScan() }
        scanRunning = false
        _isScanning.value = false
    }

    // MARK: - Connection

    /** Connects on user request; the device then auto-reconnects after
     * unexpected disconnects until explicitly disconnected. */
    suspend fun connect(id: DeviceId) {
        val facade = facade ?: return
        cancelReconnect(id)
        autoReconnect.add(id)
        try {
            facade.connect(id.canonicalJson())
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            autoReconnect.remove(id)
            _presentedError.value = PresentedError.from(error)
        }
    }

    /** Disconnects on user request and cancels any pending reconnect loop.
     * The facade's `disconnect` is idempotent. */
    suspend fun disconnect(id: DeviceId) {
        cancelReconnect(id)
        autoReconnect.remove(id)
        presentErrors { facade?.disconnect(id.canonicalJson()) }
    }

    fun dismissError() {
        _presentedError.value = null
    }

    // MARK: - Notification attribution (see KNOWN-GAPS.md)

    /** Registers the device as the notification attribution target (called
     * by the detail view when it subscribes a characteristic). */
    fun registerNotificationTarget(id: DeviceId) {
        subscribedDevices.removeAll { it == id }
        subscribedDevices.add(0, id)
    }

    /** Unregisters a notification attribution target (view gone, or all of
     * the device's subscriptions cancelled). */
    fun unregisterNotificationTarget(id: DeviceId) {
        subscribedDevices.removeAll { it == id }
    }

    // MARK: - Event intake

    private fun handleEventJson(json: String) {
        val event = try {
            json.toAdapterEvent()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.parseFailure("事件", error)
            return
        }
        apply(event)
    }

    private fun handleNotificationJson(json: String) {
        // The facade's Notification JSON carries no device identity, so the
        // demo attributes each value to the device that most recently
        // registered a subscription (the detail view is the only subscriber).
        val target = subscribedDevices.firstOrNull() ?: return
        val notification = try {
            json.toNotification()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _presentedError.value = PresentedError.parseFailure("通知", error)
            return
        }
        _notificationLogs.update { logs ->
            val log = ((logs[target] ?: emptyList()) + notification)
                .takeLast(notificationLogLimit)
            logs + (target to log)
        }
    }

    private fun apply(event: AdapterEvent) {
        when (event) {
            is AdapterEvent.StateChanged -> _adapterState.value = event.state
            is AdapterEvent.DeviceDiscovered -> _scan.update { it.apply(event.device) }
            is AdapterEvent.DeviceUpdated -> _scan.update { it.applyUpdate(event.id, event.rssi) }
            is AdapterEvent.DeviceConnected -> {
                cancelReconnect(event.id)
                _connections.update { map ->
                    map + (event.id to ConnectionState(connected = true, reconnect = null))
                }
            }
            is AdapterEvent.DeviceDisconnected -> {
                // CCCD state and the session-scoped tree die with the link;
                // logs and subscriptions are stale now. Reconnect re-issues
                // them after discovery (see DeviceDetailModel.discover).
                _notificationLogs.update { it - event.id }
                subscribedDevices.removeAll { it == event.id }
                _connections.update { map ->
                    map + (event.id to ConnectionState(
                        connected = false,
                        reconnect = map[event.id]?.reconnect,
                    ))
                }
                // The user asked for this drop - no auto-reconnect.
                if (event.id !in autoReconnect) return
                startReconnect(event.id)
            }
        }
    }

    // MARK: - Reconnect

    private fun setReconnectState(
        id: DeviceId,
        attempt: Int,
        nextDelayMillis: Long,
        lastError: String?,
    ) {
        _connections.update { map ->
            val state = map[id] ?: ConnectionState(connected = false, reconnect = null)
            map + (id to state.copy(
                reconnect = ReconnectState(attempt, nextDelayMillis, lastError),
            ))
        }
    }

    private fun cancelReconnect(id: DeviceId) {
        reconnectJobs.remove(id)?.cancel()
    }

    /** Runs the bounded-exponential-backoff reconnect loop for one device
     * until the link is re-established or the job is cancelled. The last
     * failed attempt's structured error stays on the banner while backing
     * off. */
    private fun startReconnect(id: DeviceId) {
        val facade = facade ?: return
        if (id in reconnectJobs) return
        val policy = ReconnectPolicy(baseMillis = reconnectBaseMillis)
        var lastError: String? = null
        reconnectJobs[id] = scope.launch {
            while (isActive) {
                val delayMillis = policy.nextDelayMillis()
                setReconnectState(id, policy.attempt, delayMillis, lastError)
                delay(delayMillis)
                try {
                    // Android reconnects by cached registry entry without a
                    // rescan (disconnects do not evict); a rotated RPA still
                    // requires a rescan - surfaced as NotFound then.
                    facade.connect(id.canonicalJson())
                    return@launch
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Exception) {
                    lastError = PresentedError.from(error).structured
                    setReconnectState(id, policy.attempt, delayMillis, lastError)
                }
            }
        }
    }

    // MARK: - Plumbing

    private suspend fun <T> presentErrors(block: suspend () -> T): T? = try {
        block()
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        _presentedError.value = PresentedError.from(error)
        null
    }
}
