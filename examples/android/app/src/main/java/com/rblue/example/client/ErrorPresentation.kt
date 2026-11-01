package com.rblue.example.client

import uniffi.rblue_ffi.PairingStatus
import uniffi.rblue_ffi.PermissionReason
import uniffi.rblue_ffi.RblueException

/**
 * User-facing presentation of an error, structured rather than flattened:
 * every `RblueException` case keeps its fields (raw platform codes are shown
 * verbatim, never demoted to prose), matching the facade's error contract.
 */
data class PresentedError(
    val title: String,
    val detail: String,
    val structured: String,
) {
    companion object {
        /** Android `BluetoothGatt` status 133 (GATT_ERROR): the dominant,
         * nearly content-free GATT failure code. */
        const val GATT_STATUS_133 = 133

        /** Maps any thrown error; `RblueException` exhaustively, everything
         * else (JSON decoding, local plumbing) into a generic entry with the
         * full description preserved. Callers rethrow CancellationException
         * before reaching this. */
        fun from(error: Throwable): PresentedError {
            if (error is RblueException) return fromRblue(error)
            return PresentedError(
                title = "错误",
                detail = error.toString(),
                structured = error.toString(),
            )
        }

        /** Structured presentation for contract-JSON parse failures of
         * EventSink payloads (the event is dropped, like the iOS example). */
        fun parseFailure(kind: String, error: Exception): PresentedError = PresentedError(
            title = "${kind}解析失败",
            detail = "EventSink 收到的 JSON 不符合 rblue-api 契约,已忽略。",
            structured = error.toString(),
        )

        private fun fromRblue(error: RblueException): PresentedError {
            val structured = error.message ?: error.toString()
            return when (error) {
                is RblueException.Unsupported -> PresentedError(
                    title = "不支持",
                    detail = "${error.feature} 在 ${error.platform} 上不受支持。" +
                        "这是平台事实(能力协商),不是故障;详见「平台说明」页。",
                    structured = structured,
                )
                is RblueException.Permission -> permissionPresented(error.reason, structured)
                is RblueException.NotFound -> PresentedError(
                    title = "设备未知",
                    detail = "该设备从未被扫描到,或适配器缓存中不存在。" +
                        "Android 的标识是 MAC 地址;现代外设的 RPA 约每 15 分钟轮换," +
                        "轮换后如同新设备,请重新扫描。",
                    structured = structured,
                )
                is RblueException.NotConnected -> PresentedError(
                    title = "未连接",
                    detail = "该操作需要已建立的连接;请先连接设备(重连后需重新发现服务)。",
                    structured = structured,
                )
                is RblueException.Timeout -> PresentedError(
                    title = "操作超时",
                    detail = "平台操作超过了期限,可重试。",
                    structured = structured,
                )
                is RblueException.Pairing -> PresentedError(
                    title = "配对失败",
                    detail = "状态:${render(error.status)}。Android 配对是隐式的" +
                        "(访问加密特征时由系统触发);本示例的 pair() 调用预期返回 Unsupported。",
                    structured = structured,
                )
                is RblueException.Att -> {
                    val handleText = error.handle?.let { ", handle $it" } ?: ""
                    PresentedError(
                        title = "GATT 过程失败",
                        detail = "ATT 错误码 0x${error.code.toInt().toString(16)}$handleText。" +
                            "0x01-0x7F 为协议定义码,更高为应用/Profile 码。",
                        structured = structured,
                    )
                }
                is RblueException.Platform -> platformPresented(
                    code = error.code,
                    message = error.detail,
                    structured = structured,
                )
                is RblueException.InvalidLength -> PresentedError(
                    title = "载荷超长 (InvalidLength)",
                    detail = "载荷长度 ${error.len} 超过 max_write_len ${error.max}。" +
                        "rblue 统一拒绝而不截断;原生 Android 栈会静默截断超长写——" +
                        "这是平台差异,详见「平台说明」。",
                    structured = structured,
                )
                is RblueException.Disconnected -> PresentedError(
                    title = "连接中断",
                    detail = "操作执行期间设备断开了连接。",
                    structured = structured,
                )
                is RblueException.InvalidState -> PresentedError(
                    title = "状态不允许",
                    detail = error.detail,
                    structured = structured,
                )
                is RblueException.Backend -> PresentedError(
                    title = "后端错误",
                    detail = "${error.detail}(常见原因:AAR 缺少 btleplug-java 辅助类," +
                        "或 JVM 附件丢失;详见「平台说明」页。",
                    structured = structured,
                )
                is RblueException.InvalidArgument -> PresentedError(
                    title = "参数无效",
                    detail = "${error.detail}(例如 service UUID 必须是标准连字符形式)。",
                    structured = structured,
                )
            }
        }

        /** Keeps the raw code verbatim (retry policies match on the code, not
         * on prose); 133 gets the extra context every Android BLE developer
         * eventually needs. */
        private fun platformPresented(
            code: Int,
            message: String,
            structured: String,
        ): PresentedError {
            if (code == GATT_STATUS_133) {
                return PresentedError(
                    title = "平台错误 133 (GATT_ERROR)",
                    detail = "Android GATT status 133 是最常见的控制器错误:" +
                        "连接超时、链路丢失或栈内部状态,与具体操作几乎无关。" +
                        "原始码 133 原样保留(重试策略应匹配原始码,不降级为字符串);" +
                        "fork 的 Java 层对快速失败的 133 连接尝试会自动重试至多 3 次。" +
                        "可重试,或重新扫描后重连:$message",
                    structured = structured,
                )
            }
            return PresentedError(
                title = "平台错误",
                detail = "原始状态码 $code 原样保留(重试策略应匹配原始码,不应降级为字符串):$message",
                structured = structured,
            )
        }

        private fun permissionPresented(reason: PermissionReason, structured: String) = when (reason) {
            PermissionReason.BLUETOOTH_OFF -> PresentedError(
                title = "蓝牙已关闭",
                detail = "请在系统设置或快捷开关中打开蓝牙后重试。(fork 不发 state_changed 事件," +
                    "initialize 在关蓝牙时也成功;首个操作才报告这一状态。)",
                structured = structured,
            )
            PermissionReason.LOCATION_DENIED -> PresentedError(
                title = "缺少定位权限",
                detail = "Android API 30 及以下扫描需要定位权限,且系统定位服务需开启;" +
                    "API 31+ 不需要。请在系统设置中授予「位置信息」权限。",
                structured = structured,
            )
            PermissionReason.BLUETOOTH_PERMISSION_DENIED -> PresentedError(
                title = "蓝牙权限被拒绝",
                detail = "请在 系统设置 > 应用 > 权限 中允许「附近的设备」" +
                    "(BLUETOOTH_SCAN / BLUETOOTH_CONNECT)。",
                structured = structured,
            )
            PermissionReason.UNAUTHORIZED -> PresentedError(
                title = "蓝牙权限未授权",
                detail = "系统蓝牙授权状态为 denied。请在系统设置中允许本应用使用蓝牙后重试。",
                structured = structured,
            )
        }

        private fun render(status: PairingStatus): String = when (status) {
            is PairingStatus.Rejected -> "Rejected"
            is PairingStatus.TimedOut -> "TimedOut"
            is PairingStatus.AuthenticationFailed -> "AuthenticationFailed"
            is PairingStatus.InProgress -> "InProgress"
            is PairingStatus.NotCapable -> "NotCapable"
            is PairingStatus.Other -> "Other(code=${status.code})"
        }
    }
}
