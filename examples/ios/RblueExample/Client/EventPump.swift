import Foundation
import Rblue

/// Bridges the facade's `EventSink` callbacks into `AsyncStream`s of raw
/// event JSON.
///
/// Delivery contract (crates/rblue-ffi, "Event delivery and backpressure"):
/// the Rust pump invokes the sink from worker threads; implementations must
/// not block and must not throw. Yielding into an `AsyncStream` satisfies
/// both. Buffering follows the binding guidance for Swift:
/// `.bufferingNewest` (drop-oldest, capacity 1) - a stalled consumer
/// back-pressures by dropping the oldest values instead of growing memory.
/// `AsyncStream` does not report drop counts, so the demo cannot surface
/// them (recorded in `examples/ios/KNOWN-GAPS.md`).
final class EventPump: EventSink {
    /// Serialized `AdapterEvent` JSON payloads.
    let events: AsyncStream<String>

    /// Serialized `Notification` JSON payloads.
    let notifications: AsyncStream<String>

    private let eventContinuation: AsyncStream<String>.Continuation
    private let notificationContinuation: AsyncStream<String>.Continuation

    init() {
        var eventsContinuation: AsyncStream<String>.Continuation!
        var notificationsContinuation: AsyncStream<String>.Continuation!
        events = AsyncStream(bufferingPolicy: .bufferingNewest(1)) { eventsContinuation = $0 }
        notifications = AsyncStream(bufferingPolicy: .bufferingNewest(1)) {
            notificationsContinuation = $0
        }
        eventContinuation = eventsContinuation
        notificationContinuation = notificationsContinuation
    }

    func onEvent(eventJson: String) {
        eventContinuation.yield(eventJson)
    }

    func onNotification(notificationJson: String) {
        notificationContinuation.yield(notificationJson)
    }
}
