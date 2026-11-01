import SwiftUI

/// Platform facts page (M-I3): what the CoreBluetooth backend can and cannot
/// do, how the demo's behaviors differ from the platform underneath, plus
/// live diagnostics from the bound adapter.
struct PlatformNotesView: View {
    @EnvironmentObject private var client: RblueClient

    var body: some View {
        NavigationStack {
            List {
                diagnosticsSection
                identitySection
                gattSection
                capabilitySection
                backgroundSection
                simulatorSection
            }
            .navigationTitle("平台说明")
        }
    }

    private var diagnosticsSection: some View {
        Section("适配器诊断") {
            LabeledContent("backendVersion", value: client.backendVersion)
            LabeledContent("adapterId", value: client.adapterId)
            LabeledContent("适配器状态", value: client.adapterState.localizedDescription)
            if client.capabilities.names.isEmpty {
                Text("能力集为空(模拟器路径或后端不可用)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text(client.capabilities.names.sorted().joined(separator: " · "))
                    .font(.caption.monospaced())
            }
        }
    }

    private var identitySection: some View {
        Section {
            Text("本平台只有 DeviceId::Uuid(CoreBluetooth 的 NSUUID)。")
            Text("该 UUID 在应用重装后会变化,不能当作持久设备身份;持久化时必须连同名称/服务指纹,拿不准就重新扫描。")
            Text("断连后重连不需要重新扫描:后端按标识重新取回外设(retrievePeripheralsWithIdentifiers)。")
        } header: {
            Text("设备标识")
        } footer: {
            Text("蓝牙标识属于个人数据,示例不会把设备 UUID 写入日志。")
        }
    }

    private var gattSection: some View {
        Section("GATT 与写语义") {
            Text("服务发现必须在每次连接后执行(iOS 硬性要求;标识只在当前连接内有效)。")
            Text("统一写语义:超过 max_write_len 的载荷被 InvalidLength 拒绝——不截断、不分片。")
            Text("平台差异:原生 writeValue(.withoutResponse) 会静默截断超长数据;rblue 把它变成显式错误。设备详情页的「超长写入演示」可以看到这一差异。")
            Text("通知订阅随连接死亡(CCCD 状态不跨连接):重连后必须重新 subscribe。")
            Text("断连原因:CoreBluetooth 不提供结构化原因,DisconnectReason 一律为 unknown。")
            Text("同 UUID 多实例:实例序号按 UUID 排序的发现顺序计数,而非硬件发现顺序(fork 的 BTreeSet 继承限制)。")
        }
    }

    /// Static capability matrix from
    /// crates/rblue-corebluetooth/docs/platform-notes.md, annotated with the
    /// live capability set.
    private var capabilitySection: some View {
        Section {
            ForEach(Self.capabilityMatrix, id: \.name) { row in
                HStack {
                    Text(row.name)
                        .font(.subheadline)
                    Spacer()
                    Text(row.supported ? "支持" : "不支持")
                        .font(.caption)
                        .foregroundStyle(row.supported ? .green : .secondary)
                    if client.capabilities.has(row.capabilityName) {
                        Image(systemName: "checkmark.circle")
                            .font(.caption)
                            .foregroundStyle(.green)
                    }
                }
            }
        } header: {
            Text("能力矩阵")
        } footer: {
            Text("勾选标记表示当前适配器的能力集(capabilities())包含该项;与静态矩阵不一致时请上报。")
        }
    }

    private var backgroundSection: some View {
        Section {
            Text("后台扫描需要 Info.plist 声明 bluetooth-central 后台模式,且必须携带服务 UUID 过滤,否则 iOS 静默返回空结果。")
            Text("因此后端保守地把 Capability::BackgroundScan 报告为 false;本示例不做后台扫描。")
            Text("权限:NSBluetoothAlwaysUsageDescription 触发首次授权(适配器创建时弹出);拒绝后操作返回 Permission(Unauthorized),UI 以结构化错误呈现。")
            Text("蓝牙关闭:Permission(BluetoothOff);打开蓝牙后适配器会发出 state_changed 事件。")
        } header: {
            Text("后台与权限")
        }
    }

    private var simulatorSection: some View {
        Section {
            Text("iOS 模拟器没有蓝牙硬件:构建与单元测试在模拟器/CI 完成,运行验收(扫描、连接、读写、通知、断连重连)必须真机。")
            Text("模拟器路径下本示例不启动真实后端,首屏会显示 Unsupported 的结构化错误。")
        } header: {
            Text("模拟器与验收")
        }
    }

    /// (display name, capability bit name in the JSON set, supported per the
    /// platform notes).
    private struct CapabilityRow {
        let name: String
        let capabilityName: String
        let supported: Bool
    }

    private static let capabilityMatrix: [CapabilityRow] = [
        CapabilityRow(name: "ActiveScan", capabilityName: "ACTIVE_SCAN", supported: true),
        CapabilityRow(name: "GattClient", capabilityName: "GATT_CLIENT", supported: true),
        CapabilityRow(
            name: "RssiDuringConnection",
            capabilityName: "RSSI_DURING_CONNECTION",
            supported: true
        ),
        CapabilityRow(
            name: "MtuNegotiation", capabilityName: "MTU_NEGOTIATION", supported: false
        ),
        CapabilityRow(name: "MtuQuery", capabilityName: "MTU_QUERY", supported: false),
        CapabilityRow(
            name: "RequestDevicePicker",
            capabilityName: "REQUEST_DEVICE_PICKER",
            supported: false
        ),
        CapabilityRow(name: "Pairing", capabilityName: "PAIRING", supported: false),
        CapabilityRow(
            name: "BackgroundScan", capabilityName: "BACKGROUND_SCAN", supported: false
        ),
        CapabilityRow(name: "PassiveScan", capabilityName: "PASSIVE_SCAN", supported: false),
        CapabilityRow(name: "GattServer", capabilityName: "GATT_SERVER", supported: false),
        CapabilityRow(name: "Advertise", capabilityName: "ADVERTISE", supported: false),
        CapabilityRow(
            name: "ScanResponse", capabilityName: "SCAN_RESPONSE", supported: false
        ),
    ]
}
