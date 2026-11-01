import Foundation
import Rblue

/// The subset of the generated `RblueProtocol` the app uses.
///
/// Exists so unit tests can inject a fake facade; the generated `Rblue`
/// class conforms for free because every requirement is already a
/// `RblueProtocol` witness.
protocol RblueFacade: AnyObject, Sendable {
    func initialize() async throws
    func adapterId() -> String
    func backendVersion() -> String
    func capabilities() -> String
    func startScan(filterJson: String) async throws
    func stopScan() async throws
    func connect(deviceJson: String) async throws
    func disconnect(deviceJson: String) async throws
    func discoverServices(deviceJson: String, serviceUuids: [String]?) async throws -> String
    func read(deviceJson: String, characteristicJson: String) async throws -> Data
    func write(
        deviceJson: String, characteristicJson: String, data: Data, withResponse: Bool
    ) async throws
    func maxWriteLen(
        deviceJson: String, characteristicJson: String, withResponse: Bool
    ) async throws -> UInt64
    func subscribe(deviceJson: String, characteristicJson: String) async throws
    func unsubscribe(deviceJson: String, characteristicJson: String) async throws
    func rssi(deviceJson: String) async throws -> Int16
    func mtu(deviceJson: String) async throws -> UInt16
    func name(deviceJson: String) async -> String?
    func pair(deviceJson: String) async throws
    func setEventSink(sink: EventSink)
}

extension Rblue: RblueFacade {}
