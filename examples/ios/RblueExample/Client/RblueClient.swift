import Foundation
import Rblue

/// The app's single façade owner: installs the `EventSink`, converts the
/// raw JSON event streams into typed state (`ScanListModel`, connection
/// bookkeeping, notification logs), and runs the Swift-side automatic
/// reconnect (the facade does not export a supervisor, so reconnect policy
/// lives here - same as the Android example).
///
/// All mutable state is `@MainActor`; the pump's callbacks only yield into
/// `AsyncStream`s (never blocking, never throwing), and the consumption
/// tasks hop back to the main actor to apply events.
@MainActor
final class RblueClient: ObservableObject {
    /// Per-device reconnect bookkeeping shown in the UI.
    struct ReconnectState: Equatable {
        var attempt: Int
        var nextDelayMilliseconds: Int
        /// Structured rendering of the last failed attempt, if any.
        var lastError: String?
    }

    /// Connection bookkeeping per device.
    struct ConnectionState: Equatable {
        var connected: Bool
        var reconnect: ReconnectState?
    }

    /// Upper bound of the per-device notification log (demo UI only).
    static let notificationLogLimit = 100

    // Diagnostics captured from the facade at construction time (sync calls).
    private(set) var backendVersion = "unavailable"
    private(set) var adapterId = "unavailable"
    private(set) var capabilities = CapabilitySet("0x0")

    @Published private(set) var adapterState: AdapterState = .unknown
    @Published private(set) var scan = ScanListModel()
    @Published private(set) var connections: [DeviceId: ConnectionState] = [:]
    @Published private(set) var isScanning = false
    /// Newest-last per-device notification log, capped at
    /// `notificationLogLimit`.
    @Published private(set) var notificationLogs: [DeviceId: [Notification]] = [:]
    @Published private(set) var presentedError: PresentedError?

    private let pump = EventPump()
    private var facade: (any RblueFacade)?
    private var eventTask: Task<Void, Never>?
    private var notificationTask: Task<Void, Never>?
    private var reconnectTasks: [DeviceId: Task<Void, Never>] = [:]
    /// Devices the user connected through this client and has not explicitly
    /// disconnected; these auto-reconnect on unexpected disconnects.
    private var autoReconnect: Set<DeviceId> = []
    /// Devices with at least one active subscription, most recent first.
    /// The facade's `Notification` JSON carries no device identity, so
    /// attribution uses this registry (see KNOWN-GAPS.md).
    private var subscribedDevices: [DeviceId] = []
    private var scanRunning = false
    private var reconnectBaseMilliseconds = 500

    private static let decoder = JSONDecoder()

    /// The default facade factory.
    ///
    /// The iOS Simulator has no Bluetooth radio; booting the real backend
    /// there would only yield "radio unavailable" noise (and CI runs unit
    /// tests against the simulator), so on the simulator the client starts
    /// without a facade and the UI shows the real-device hint instead.
    /// `nonisolated` because default argument expressions are evaluated
    /// outside the actor.
    nonisolated static var defaultFacadeFactory: () throws -> any RblueFacade {
        #if targetEnvironment(simulator)
            {
                throw RblueError.Unsupported(
                    platform: "ios-simulator",
                    feature: "Bluetooth radio (simulators have no BLE hardware)"
                )
            }
        #else
            { try Rblue() }
        #endif
    }

    /// # Parameters
    ///
    /// - `facadeFactory` - injectable for unit tests; defaults to the real
    ///   facade (or the no-facade simulator path above).
    /// - `reconnectBaseMilliseconds` - injectable so reconnect tests do not
    ///   wait half a second per attempt.
    init(
        facadeFactory: () throws -> any RblueFacade = RblueClient.defaultFacadeFactory,
        reconnectBaseMilliseconds: Int = 500
    ) {
        self.reconnectBaseMilliseconds = reconnectBaseMilliseconds
        do {
            let facade = try facadeFactory()
            self.facade = facade
            self.backendVersion = facade.backendVersion()
            self.adapterId = facade.adapterId()
            self.capabilities = CapabilitySet(facade.capabilities())
        } catch {
            self.facade = nil
            self.presentedError = .from(error)
        }
    }

    deinit {
        eventTask?.cancel()
        notificationTask?.cancel()
        for (_, task) in reconnectTasks {
            task.cancel()
        }
    }

    /// The facade when one is available; `nil` on the simulator path.
    var activeFacade: (any RblueFacade)? {
        facade
    }

    func isConnected(_ id: DeviceId) -> Bool {
        connections[id]?.connected ?? false
    }

    func reconnectState(for id: DeviceId) -> ReconnectState? {
        connections[id]?.reconnect
    }

    // MARK: - Startup

    /// Installs the event sink and starts consuming both streams. Must run
    /// before `initializeAdapter` so state events are not missed. Idempotent.
    func start() {
        guard eventTask == nil, let facade else { return }
        facade.setEventSink(sink: pump)
        eventTask = Task { [weak self] in
            guard let pump = self?.pump else { return }
            for await json in pump.events {
                guard let self else { break }
                self.handleEventJson(json)
            }
        }
        notificationTask = Task { [weak self] in
            guard let pump = self?.pump else { return }
            for await json in pump.notifications {
                guard let self else { break }
                self.handleNotificationJson(json)
            }
        }
    }

    /// Initializes the adapter. On iOS this is where the Bluetooth
    /// authorization prompt fires (adapter creation triggers it).
    func initializeAdapter() async {
        guard let facade else { return }
        do {
            try await facade.initialize()
        } catch {
            presentedError = .from(error)
        }
    }

    // MARK: - Scan

    /// Starts a scan with the optional filters; rows are cleared so stale
    /// devices from a previous filter do not linger.
    func startScan(namePrefix: String?, serviceUuid: String?, rssiThreshold: Int16?) async {
        guard let facade, !scanRunning else { return }
        let filter = ScanFilter(
            namePrefix: namePrefix.flatMap {
                let trimmed = $0.trimmingCharacters(in: .whitespacesAndNewlines)
                return trimmed.isEmpty ? nil : trimmed
            },
            services: serviceUuid.map { [$0] } ?? [],
            rssiThreshold: rssiThreshold
        )
        do {
            try await facade.startScan(filterJson: filter.jsonEncoded())
            scanRunning = true
            isScanning = true
            scan.clear()
        } catch {
            presentedError = .from(error)
        }
    }

    /// Stops the running scan. Idempotent.
    func stopScan() async {
        guard let facade else { return }
        do {
            try await facade.stopScan()
        } catch {
            presentedError = .from(error)
        }
        scanRunning = false
        isScanning = false
    }

    // MARK: - Connection

    /// Connects on user request; the device then auto-reconnects after
    /// unexpected disconnects until explicitly disconnected.
    func connect(_ id: DeviceId) async {
        guard let facade else { return }
        reconnectTasks[id]?.cancel()
        reconnectTasks[id] = nil
        autoReconnect.insert(id)
        do {
            try await facade.connect(deviceJson: try id.deviceJson())
        } catch {
            autoReconnect.remove(id)
            presentedError = .from(error)
        }
    }

    /// Disconnects on user request and cancels any pending reconnect loop.
    /// The facade's `disconnect` is idempotent.
    func disconnect(_ id: DeviceId) async {
        guard let facade else { return }
        reconnectTasks[id]?.cancel()
        reconnectTasks[id] = nil
        autoReconnect.remove(id)
        do {
            try await facade.disconnect(deviceJson: try id.deviceJson())
        } catch {
            presentedError = .from(error)
        }
    }

    func dismissError() {
        presentedError = nil
    }

    // MARK: - Event intake

    private func handleEventJson(_ json: String) {
        do {
            apply(try Self.decoder.decode(AdapterEvent.self, from: Data(json.utf8)))
        } catch {
            presentedError = PresentedError(
                title: "事件解析失败",
                detail: "EventSink 收到的 JSON 不符合 rblue-api 契约,已忽略该事件。",
                structured: String(describing: error)
            )
        }
    }

    private func handleNotificationJson(_ json: String) {
        // The facade's Notification JSON carries no device identity, so the
        // demo attributes each value to the device that most recently
        // registered a subscription (the detail view is the only subscriber;
        // see KNOWN-GAPS.md).
        guard let target = subscribedDevices.first else { return }
        do {
            let notification = try Self.decoder.decode(Notification.self, from: Data(json.utf8))
            var log = notificationLogs[target] ?? []
            log.append(notification)
            if log.count > Self.notificationLogLimit {
                log.removeFirst(log.count - Self.notificationLogLimit)
            }
            notificationLogs[target] = log
        } catch {
            presentedError = PresentedError(
                title: "通知解析失败",
                detail: "EventSink 收到的通知 JSON 不符合 rblue-api 契约,已忽略。",
                structured: String(describing: error)
            )
        }
    }

    /// Registers the device as the notification attribution target (called
    /// by the detail view when it subscribes a characteristic).
    func registerNotificationTarget(_ id: DeviceId) {
        subscribedDevices.removeAll { $0 == id }
        subscribedDevices.insert(id, at: 0)
    }

    /// Unregisters a notification attribution target (view gone, or all of
    /// the device's subscriptions cancelled).
    func unregisterNotificationTarget(_ id: DeviceId) {
        subscribedDevices.removeAll { $0 == id }
    }

    private func apply(_ event: AdapterEvent) {
        switch event {
        case .stateChanged(let state):
            adapterState = state
        case .deviceDiscovered(let discovered):
            scan.apply(discovered: discovered)
        case .deviceUpdated(let id, let rssi):
            scan.apply(updateFor: id, rssi: rssi)
        case .deviceConnected(let id):
            connections[id] = ConnectionState(connected: true, reconnect: nil)
            reconnectTasks[id]?.cancel()
            reconnectTasks[id] = nil
        case .deviceDisconnected(let id, _):
            // CoreBluetooth reports no structured reason (always `.unknown`
            // here), so the reason is not displayed; see the platform notes
            // page. CCCD state dies with the connection (CoreBluetooth drops
            // it), so subscriptions and notification logs are stale now.
            notificationLogs[id] = nil
            subscribedDevices.removeAll { $0 == id }
            let reconnected = ConnectionState(
                connected: false,
                reconnect: connections[id]?.reconnect
            )
            connections[id] = reconnected
            // The user asked for this drop - no auto-reconnect.
            if !autoReconnect.contains(id) {
                return
            }
            startReconnect(for: id)
        }
    }

    private func setReconnectState(
        _ id: DeviceId, attempt: Int, nextDelayMilliseconds: Int, lastError: String? = nil
    ) {
        var state = connections[id] ?? ConnectionState(connected: false, reconnect: nil)
        state.reconnect = ReconnectState(
            attempt: attempt,
            nextDelayMilliseconds: nextDelayMilliseconds,
            lastError: lastError
        )
        connections[id] = state
    }

    /// Runs the bounded-exponential-backoff reconnect loop for one device
    /// until the link is re-established or the task is cancelled. The last
    /// failed attempt's structured error stays on the banner while backing
    /// off.
    private func startReconnect(for id: DeviceId) {
        guard reconnectTasks[id] == nil, facade != nil else { return }
        var policy = ReconnectPolicy(baseMilliseconds: reconnectBaseMilliseconds)
        var lastReportedError: String?
        let task = Task { [weak self] in
            while !Task.isCancelled {
                let delay = policy.nextDelayMilliseconds()
                self?.setReconnectState(
                    id,
                    attempt: policy.attempt,
                    nextDelayMilliseconds: delay,
                    lastError: lastReportedError
                )
                do {
                    try await Task.sleep(for: .milliseconds(delay))
                } catch {
                    return
                }
                guard let self, let facade = self.facade else { return }
                do {
                    // CoreBluetooth reconnects by identifier without a
                    // rescan; the backend re-retrieves the peripheral.
                    try await facade.connect(deviceJson: try id.deviceJson())
                    return
                } catch is CancellationError {
                    return
                } catch {
                    lastReportedError = PresentedError.from(error).structured
                    self.setReconnectState(
                        id,
                        attempt: policy.attempt,
                        nextDelayMilliseconds: delay,
                        lastError: lastReportedError
                    )
                }
            }
        }
        reconnectTasks[id] = task
    }
}
