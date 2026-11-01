package com.rblue.example.client

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Structured error presentation: raw platform codes preserved verbatim
 * (never demoted to prose), the GATT 133 special case, and the permission
 * reason mapping. */
class ErrorPresentationTest {
    @Test
    fun platform133GetsSpecialTextWithRawCodePreserved() {
        val presented = PresentedError.from(
            uniffi.rblue_ffi.RblueException.Platform(133, "connect failed"),
        )
        assertEquals("平台错误 133 (GATT_ERROR)", presented.title)
        assertTrue(presented.structured.contains("133"))
        assertTrue(presented.detail.contains("133"))
    }

    @Test
    fun otherPlatformCodesStayGenericButVerbatim() {
        val presented = PresentedError.from(
            uniffi.rblue_ffi.RblueException.Platform(8, "timeout"),
        )
        assertEquals("平台错误", presented.title)
        assertTrue(presented.detail.contains("8"))
        assertTrue(presented.detail.contains("timeout"))
        assertEquals("code=8, detail=timeout", presented.structured)
    }

    @Test
    fun invalidLengthKeepsStructuredFields() {
        val presented = PresentedError.from(
            uniffi.rblue_ffi.RblueException.InvalidLength(513u, 512u),
        )
        assertEquals("载荷超长 (InvalidLength)", presented.title)
        assertTrue(presented.detail.contains("513"))
        assertTrue(presented.detail.contains("512"))
        assertEquals("len=513, max=512", presented.structured)
    }

    @Test
    fun permissionReasonsMapToDedicatedText() {
        val off = PresentedError.from(
            uniffi.rblue_ffi.RblueException.Permission(uniffi.rblue_ffi.PermissionReason.BLUETOOTH_OFF),
        )
        assertEquals("蓝牙已关闭", off.title)
        val location = PresentedError.from(
            uniffi.rblue_ffi.RblueException.Permission(uniffi.rblue_ffi.PermissionReason.LOCATION_DENIED),
        )
        assertEquals("缺少定位权限", location.title)
        val denied = PresentedError.from(
            uniffi.rblue_ffi.RblueException.Permission(
                uniffi.rblue_ffi.PermissionReason.BLUETOOTH_PERMISSION_DENIED,
            ),
        )
        assertEquals("蓝牙权限被拒绝", denied.title)
    }

    @Test
    fun nonFacadeErrorsFallBackToGenericPresentation() {
        val presented = PresentedError.from(IllegalStateException("boom"))
        assertEquals("错误", presented.title)
        assertTrue(presented.structured.contains("boom"))
    }
}
