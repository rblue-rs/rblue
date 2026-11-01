package com.rblue.example.client

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import uniffi.rblue_ffi.EventSink
import uniffi.rblue_ffi.Rblue

/** One message multiplexed out of the facade's event sink: an adapter event
 * or a GATT notification, each as raw contract JSON. */
sealed interface PumpMessage {
    data class Event(val json: String) : PumpMessage
    data class Notification(val json: String) : PumpMessage
}

/**
 * Bridges the facade's `EventSink` callbacks into a
 * `callbackFlow(BUFFERED)` of raw JSON messages.
 *
 * Delivery contract (crates/rblue-ffi): the Rust pump invokes the sink from
 * worker threads; implementations must not block, must not re-enter the
 * facade, and must not throw. `trySend` on the buffered channel satisfies
 * all three: it never suspends the pump thread. When the buffer is full the
 * new message is dropped (drop-newest; the alternative - blocking - would
 * stall the pump). The flow must be collected exactly once; see
 * [RblueClient.start].
 */
class EventPump(private val facade: Rblue) {
    val messages: Flow<PumpMessage> = callbackFlow {
        val sink = object : EventSink {
            override fun onEvent(eventJson: String) {
                trySend(PumpMessage.Event(eventJson))
            }

            override fun onNotification(notificationJson: String) {
                trySend(PumpMessage.Notification(notificationJson))
            }
        }
        // Installs (or replaces) the process-wide sink. The Rust pump starts
        // with the first install and keeps channel and forwarders running
        // across replacements; values subscribed before this call flush
        // once the pump observes the sink.
        facade.setEventSink(sink)
        awaitClose {
            // Nothing to release: the sink stays installed for the facade's
            // lifetime (set_event_sink has no uninstall counterpart).
        }
    }.buffer(Channel.BUFFERED)
}
