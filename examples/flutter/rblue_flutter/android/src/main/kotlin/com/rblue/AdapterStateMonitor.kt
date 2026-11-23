package com.rblue

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build

/**
 * Delivers live Bluetooth adapter state to the rblue-android shell.
 *
 * The btleplug fork does not track radio state, so the shell registers this
 * receiver (and a one-shot `getState()` read) after JNI init. State codes
 * are `BluetoothAdapter` constants; `null` adapter is reported as `-2` and
 * a `SecurityException` from `getState()` as `-3`.
 */
object AdapterStateMonitor {
    @Volatile
    private var started = false

    /**
     * Registers a process-wide receiver. Idempotent. [context] is only used
     * to obtain the application context.
     */
    @Synchronized
    fun start(context: Context) {
        if (started) {
            reportCurrentState()
            return
        }
        val app = context.applicationContext
        val filter = IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED)
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                if (intent?.action != BluetoothAdapter.ACTION_STATE_CHANGED) {
                    return
                }
                val state = intent.getIntExtra(
                    BluetoothAdapter.EXTRA_STATE,
                    BluetoothAdapter.ERROR,
                )
                RblueNative.nativeOnAdapterStateChanged(state)
            }
        }
        if (Build.VERSION.SDK_INT >= 33) {
            app.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            app.registerReceiver(receiver, filter)
        }
        started = true
        reportCurrentState()
    }

    @SuppressLint("MissingPermission")
    private fun reportCurrentState() {
        val adapter = BluetoothAdapter.getDefaultAdapter()
        if (adapter == null) {
            RblueNative.nativeOnAdapterStateChanged(-2)
            return
        }
        try {
            RblueNative.nativeOnAdapterStateChanged(adapter.state)
        } catch (_: SecurityException) {
            RblueNative.nativeOnAdapterStateChanged(-3)
        }
    }
}
