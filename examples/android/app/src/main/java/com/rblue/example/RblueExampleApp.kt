package com.rblue.example

import android.app.Application
import com.rblue.RblueAndroid
import com.rblue.example.client.RblueClient

/**
 * Process owner of the client.
 *
 * Initialization discipline (crates/rblue-android/docs/platform-notes.md):
 * [RblueAndroid.initialize] must run on an application thread that can see
 * the application `ClassLoader` - the main thread here - before the first
 * `Rblue()` construction, and never from a static initializer. The call is
 * idempotent; a failure throws with a descriptive message (typically a
 * stripped AAR payload), which crashes at startup instead of surfacing later
 * as an opaque backend error.
 */
class RblueExampleApp : Application() {
    lateinit var client: RblueClient
        private set

    override fun onCreate() {
        super.onCreate()
        RblueAndroid.initialize(this)
        client = RblueClient()
    }
}
