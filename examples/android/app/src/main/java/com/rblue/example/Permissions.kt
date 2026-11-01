package com.rblue.example

import android.Manifest
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat

/**
 * Runtime-permission management for BLE (M-A3): API 31+ requires the
 * Bluetooth runtime permissions, API 30 and below require location for
 * scanning. Denials surface later as structured `Permission(...)` errors
 * from the facade - this class only gates the UI.
 */
object BlePermissions {
    const val BLUETOOTH_SCAN = Manifest.permission.BLUETOOTH_SCAN
    const val BLUETOOTH_CONNECT = Manifest.permission.BLUETOOTH_CONNECT
    const val ACCESS_FINE_LOCATION = Manifest.permission.ACCESS_FINE_LOCATION

    /** Runtime permissions the app must hold for BLE, by platform API level
     * (injectable so the branch logic is unit-testable off-device). */
    fun required(sdkInt: Int): List<String> =
        if (sdkInt >= 31) {
            listOf(BLUETOOTH_SCAN, BLUETOOTH_CONNECT)
        } else {
            listOf(ACCESS_FINE_LOCATION)
        }

    /** The required permissions not yet granted in this process. */
    fun missing(context: Context): List<String> =
        required(Build.VERSION.SDK_INT).filter {
            ContextCompat.checkSelfPermission(context, it) != PackageManager.PERMISSION_GRANTED
        }

    /** Whether the system Bluetooth radio is on (false when the manager or
     * adapter is absent, e.g. the emulator's real flavor). */
    fun isBluetoothEnabled(context: Context): Boolean {
        val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
        return manager?.adapter?.isEnabled == true
    }
}
