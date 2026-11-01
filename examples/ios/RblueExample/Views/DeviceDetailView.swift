import SwiftUI

/// Device detail: connection header with the reconnect banner, the
/// platform-limitation probes, and the discovered service/characteristic
/// tree with read/write/subscribe actions.
struct DeviceDetailView: View {
    @EnvironmentObject private var client: RblueClient
    let device: DeviceId

    @State private var model: DeviceDetailModel?

    var body: some View {
        Group {
            if let model {
                detailList(model)
            } else {
                ProgressView("准备中")
            }
        }
        .navigationTitle(model?.deviceName ?? "设备")
        .task {
            if model == nil {
                model = DeviceDetailModel(device: device, client: client)
            }
            await model?.discover()
        }
        .onDisappear {
            Task { await model?.unsubscribeAll() }
        }
        .onChange(of: client.isConnected(device)) { connected in
            // Reconnect re-established the link: discovery is mandatory on
            // every connection, and kept subscriptions are re-issued.
            guard connected else { return }
            Task { await model?.discover() }
        }
        .modifier(ModelErrorAlert(model: $model))
    }

    private func detailList(_ model: DeviceDetailModel) -> some View {
        List {
            headerSection(model)
            if client.isConnected(device) {
                probeSection(model)
                treeSection(model)
            }
        }
    }

    private func headerSection(_ model: DeviceDetailModel) -> some View {
        Section("连接") {
            LabeledContent("名称", value: model.deviceName ?? "(未知)")
            LabeledContent("标识", value: device.summary)
            LabeledContent(
                "状态",
                value: client.isConnected(device) ? "已连接" : "未连接"
            )
            if let rssi = model.rssi {
                LabeledContent("RSSI", value: "\(rssi) dBm")
            }
            if let reconnect = client.reconnectState(for: device) {
                VStack(alignment: .leading, spacing: 4) {
                    Label(
                        "自动重连:第 \(reconnect.attempt) 次尝试,"
                            + "约 \(reconnect.nextDelayMilliseconds) ms 后",
                        systemImage: "arrow.triangle.2.circlepath"
                    )
                    .foregroundStyle(.orange)
                    if let lastError = reconnect.lastError {
                        Text(lastError)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Button(client.isConnected(device) ? "断开连接" : "连接") {
                Task {
                    if client.isConnected(device) {
                        await client.disconnect(device)
                    } else {
                        await client.connect(device)
                    }
                }
            }
            if model.isDiscovering {
                LabeledContent("服务发现", value: "进行中…")
            }
        }
    }

    /// Live probes of operations iOS does not support - each button is
    /// expected to surface the structured `Unsupported` error, not crash.
    private func probeSection(_ model: DeviceDetailModel) -> some View {
        Section {
            HStack {
                Button("mtu() 探针") {
                    Task { await model.probeMtu() }
                }
                Spacer()
                Button("pair() 探针") {
                    Task { await model.probePair() }
                }
            }
        } header: {
            Text("平台限制探针")
        } footer: {
            Text("iOS 不暴露 MTU、配对是隐式的:这两个调用预期返回 Unsupported,并以结构化错误呈现。")
        }
    }

    private func treeSection(_ model: DeviceDetailModel) -> some View {
        Section {
            if model.services.isEmpty {
                Text(model.isDiscovering ? "发现中…" : "尚未发现服务")
                    .foregroundStyle(.secondary)
            }
            ForEach(model.services) { service in
                ServiceDisclosure(service: service, model: model)
            }
        } header: {
            Text("服务与特征(\(model.services.count) 个服务)")
        } footer: {
            Text(
                "特征标识是会话级的:重连后必须重新发现服务。"
                + "「超长写入」演示统一写语义 - rblue 以 InvalidLength 拒绝,"
                + "而原生 CoreBluetooth 会静默截断 without-response 写。"
            )
        }
    }
}

/// One service with its characteristics (collapsed by default keeps long
/// trees navigable).
private struct ServiceDisclosure: View {
    let service: DeviceDetailModel.ServiceNode
    @ObservedObject var model: DeviceDetailModel
    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            ForEach(service.characteristics) { node in
                CharacteristicRow(node: node, service: service, model: model)
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(service.uuid)
                    .font(.subheadline)
                Text(service.primary ? "primary · \(service.characteristics.count) 个特征"
                    : "secondary · \(service.characteristics.count) 个特征")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// One characteristic with its actions: read, write (both modes), the
/// oversize-write demonstration, and the notification log.
private struct CharacteristicRow: View {
    let node: DeviceDetailModel.CharacteristicNode
    let service: DeviceDetailModel.ServiceNode
    @ObservedObject var model: DeviceDetailModel
    @EnvironmentObject private var client: RblueClient

    @State private var writeText = ""
    @State private var withResponse = true
    @State private var expanded = false

    private var busy: Bool {
        model.busyCharacteristicIds.contains(node.id)
    }

    private var recentNotifications: [(offset: Int, element: Notification)] {
        Array(
            model.notifications
                .filter { $0.id.uuid == node.uuid && $0.id.service.uuid == service.uuid }
                .suffix(5)
                .reversed()
                .enumerated()
        )
    }

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            propertySection
            valueSection
            actionSection
            if node.subscribed {
                notificationSection
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(node.uuid)
                    .font(.caption.monospaced())
                HStack(spacing: 6) {
                    Text(node.propertyBadges.joined(separator: " "))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if node.subscribed {
                        Text("已订阅")
                            .font(.caption2)
                            .foregroundStyle(.green)
                    }
                }
            }
        }
    }

    private var propertySection: some View {
        LabeledContent(
            "属性",
            value: node.propertyBadges.isEmpty ? "无" : node.propertyBadges.joined(separator: " ")
        )
    }

    private var valueSection: some View {
        Group {
            if let value = node.value {
                VStack(alignment: .leading, spacing: 2) {
                    LabeledContent("当前值", value: "\(value.count) 字节")
                    Text(DataRendering.hex(value))
                        .font(.caption.monospaced())
                    Text(DataRendering.text(value))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else {
                LabeledContent("当前值", value: "尚未读取")
            }
            if let lastAction = node.lastAction {
                Text(lastAction)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var actionSection: some View {
        Group {
            if node.canRead {
                Button("读取") {
                    Task { await model.read(node) }
                }
                .disabled(busy)
            }
            if node.canWrite || node.canWriteWithoutResponse {
                TextField("写入文本载荷", text: $writeText)
                    .autocorrectionDisabled()
                    .font(.caption)
                Picker("写入模式", selection: $withResponse) {
                    Text("With response").tag(true)
                    Text("Without response").tag(false)
                }
                .pickerStyle(.segmented)
                HStack {
                    Button("写入") {
                        let payload = Data(writeText.utf8)
                        Task {
                            await model.write(node, payload: payload, withResponse: withResponse)
                        }
                    }
                    .disabled(busy || writeText.isEmpty)
                    Button("超长写入演示") {
                        Task { await model.writeOversizeDemo(node, withResponse: withResponse) }
                    }
                    .disabled(busy)
                }
            }
            if node.canNotify || node.canIndicate {
                Button(node.subscribed ? "取消订阅" : "订阅") {
                    Task { await model.toggleSubscription(node) }
                }
                .disabled(busy)
            }
        }
    }

    private var notificationSection: some View {
        Group {
            LabeledContent(
                "通知(最近 5 条)",
                value: "\(model.notifications.count) 条"
            )
            ForEach(recentNotifications, id: \.offset) { entry in
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(entry.element.data.count) 字节 · \(DataRendering.hex(Data(entry.element.data)))")
                        .font(.caption2.monospaced())
                    Text(DataRendering.text(Data(entry.element.data)))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}
