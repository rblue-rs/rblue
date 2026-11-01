package com.rblue.example

import org.junit.Assert.assertEquals
import org.junit.Test

/** The API-level permission branch of the BLE gate (pure logic; the runtime
 * grant checks are Android-instrumented and verified on a device). */
class BlePermissionsTest {
    @Test
    fun api30AndBelowRequireLocationForScanning() {
        assertEquals(
            listOf(BlePermissions.ACCESS_FINE_LOCATION),
            BlePermissions.required(sdkInt = 24),
        )
        assertEquals(
            listOf(BlePermissions.ACCESS_FINE_LOCATION),
            BlePermissions.required(sdkInt = 30),
        )
    }

    @Test
    fun api31AndAboveRequireBluetoothRuntimePermissions() {
        assertEquals(
            listOf(BlePermissions.BLUETOOTH_SCAN, BlePermissions.BLUETOOTH_CONNECT),
            BlePermissions.required(sdkInt = 31),
        )
        assertEquals(
            listOf(BlePermissions.BLUETOOTH_SCAN, BlePermissions.BLUETOOTH_CONNECT),
            BlePermissions.required(sdkInt = 35),
        )
    }
}
