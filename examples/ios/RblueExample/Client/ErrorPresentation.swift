import Foundation
import Rblue

/// User-facing presentation of an error, structured rather than flattened:
/// every `RblueError` case keeps its fields (raw platform codes are shown
/// verbatim, never demoted to prose), matching the facade's error contract.
struct PresentedError: Identifiable, Equatable {
    let id = UUID()
    let title: String
    let detail: String
    /// Raw structured rendering of the original error, e.g.
    /// `InvalidLength(len: 513, max: 512)`.
    let structured: String

    /// Maps any thrown error; `RblueError` exhaustively, everything else
    /// (JSON decoding, local plumbing) into a generic entry with the full
    /// description preserved.
    static func from(_ error: Error) -> PresentedError {
        guard let rblueError = error as? RblueError else {
            return PresentedError(
                title: "错误",
                detail: String(describing: error),
                structured: String(describing: error)
            )
        }
        return fromRblue(rblueError)
    }

    private static func fromRblue(_ error: RblueError) -> PresentedError {
        let structured = String(describing: error)
        switch error {
        case .Unsupported(let platform, let feature):
            return PresentedError(
                title: "不支持",
                detail: "\(feature) 在 \(platform) 上不受支持。这是平台事实(能力协商),不是故障;详见「平台说明」页。",
                structured: structured
            )
        case .Permission(let reason):
            return permissionPresented(reason: reason, structured: structured)
        case .NotFound:
            return PresentedError(
                title: "设备未知",
                detail: "该设备从未被扫描到,或适配器缓存中不存在。在 iOS 上标识是安装级 UUID,重装应用后会变化。",
                structured: structured
            )
        case .NotConnected:
            return PresentedError(
                title: "未连接",
                detail: "该操作需要已建立的连接;请先连接设备(重连后需重新发现服务)。",
                structured: structured
            )
        case .Timeout:
            return PresentedError(
                title: "操作超时",
                detail: "平台操作超过了期限,可重试。",
                structured: structured
            )
        case .Pairing(let status):
            return PresentedError(
                title: "配对失败",
                detail: "状态:\(String(describing: status))。iOS 配对是隐式的(访问加密特征时触发),本示例的 pair() 调用预期返回 Unsupported。",
                structured: structured
            )
        case .Att(let code, let handle):
            let handleText = handle.map { ", handle \($0)" } ?? ""
            return PresentedError(
                title: "GATT 过程失败",
                detail: "ATT 错误码 0x\(String(code, radix: 16))\(handleText)。0x01-0x7F 为协议定义码,更高为应用/Profile 码。",
                structured: structured
            )
        case .Platform(let code, let detail):
            return PresentedError(
                title: "平台错误",
                detail: "原始状态码 \(code) 原样保留(重试策略应匹配原始码,不应降级为字符串):\(detail)",
                structured: structured
            )
        case .InvalidLength(let len, let max):
            return PresentedError(
                title: "载荷超长 (InvalidLength)",
                detail: "载荷长度 \(len) 超过 max_write_len \(max)。rblue 统一拒绝而不截断;原生 CoreBluetooth 在 without-response 写时会静默截断——这是平台差异,详见「平台说明」。",
                structured: structured
            )
        case .Disconnected:
            return PresentedError(
                title: "连接中断",
                detail: "操作执行期间设备断开了连接。",
                structured: structured
            )
        case .InvalidState(let detail):
            return PresentedError(
                title: "状态不允许",
                detail: detail,
                structured: structured
            )
        case .Backend(let detail):
            return PresentedError(
                title: "后端错误",
                detail: detail,
                structured: structured
            )
        case .InvalidArgument(let detail):
            return PresentedError(
                title: "参数无效",
                detail: "\(detail)(例如 service UUID 必须是标准连字符形式)。",
                structured: structured
            )
        }
    }

    private static func permissionPresented(
        reason: PermissionReason, structured: String
    ) -> PresentedError {
        switch reason {
        case .bluetoothOff:
            return PresentedError(
                title: "蓝牙已关闭",
                detail: "请在控制中心或系统设置中打开蓝牙后重试。",
                structured: structured
            )
        case .locationDenied:
            return PresentedError(
                title: "缺少定位权限",
                detail: "iOS 不需要位置权限即可使用蓝牙;该状态在此平台不应出现。",
                structured: structured
            )
        case .bluetoothPermissionDenied:
            return PresentedError(
                title: "蓝牙权限被拒绝",
                detail: "请在 系统设置 > 隐私与安全性 > 蓝牙 中允许本应用。",
                structured: structured
            )
        case .unauthorized:
            return PresentedError(
                title: "蓝牙权限未授权",
                detail: "系统蓝牙授权状态为 denied。请在 系统设置 > 隐私与安全性 > 蓝牙 中允许本应用后重试。",
                structured: structured
            )
        }
    }
}
