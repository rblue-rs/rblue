import SwiftUI

/// Scan screen: adapter state banner, filter inputs, and the deduplicated
/// discovery list sorted by RSSI.
struct ScanView: View {
    @EnvironmentObject private var client: RblueClient
    @State private var namePrefix = ""
    @State private var serviceUuid = ""
    @State private var rssiThreshold = ""
    @State private var thresholdInvalid = false

    var body: some View {
        NavigationStack {
            List {
                adapterSection
                filterSection
                deviceSection
            }
            .navigationDestination(for: DeviceId.self) { device in
                DeviceDetailView(device: device)
            }
            .navigationTitle("扫描")
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button {
                        Task { await toggleScan() }
                    } label: {
                        Image(systemName: client.isScanning ? "stop.circle.fill" : "play.circle")
                    }
                    .disabled(client.activeFacade == nil)
                }
            }
        }
    }

    private var adapterSection: some View {
        Section("适配器") {
            LabeledContent("状态", value: client.adapterState.localizedDescription)
            LabeledContent("backendVersion", value: client.backendVersion)
            if client.adapterState != .poweredOn {
                Button("重新初始化") {
                    Task { await client.initializeAdapter() }
                }
            }
            #if targetEnvironment(simulator)
                Label(
                    "iOS 模拟器没有蓝牙硬件;本示例的运行验收必须在真机上完成。",
                    systemImage: "exclamationmark.triangle"
                )
            #endif
        }
    }

    private var filterSection: some View {
        Section("过滤(可选)") {
            TextField("名称前缀", text: $namePrefix)
                .autocorrectionDisabled()
            TextField("服务 UUID(如 180F / 完整 UUID)", text: $serviceUuid)
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
            TextField("RSSI 阈值 dBm(如 -80)", text: $rssiThreshold)
                .keyboardType(.numbersAndPunctuation)
            if thresholdInvalid {
                Label("RSSI 阈值必须是整数 dBm", systemImage: "xmark.circle")
                    .foregroundStyle(.red)
            }
        }
    }

    private var deviceSection: some View {
        Section("设备(\(client.scan.entries.count))") {
            if client.scan.entries.isEmpty {
                Text(client.isScanning ? "扫描中…" : "点右上角开始扫描")
                    .foregroundStyle(.secondary)
            }
            ForEach(client.scan.sortedByRssi) { entry in
                NavigationLink(value: entry.id) {
                    ScanRow(entry: entry)
                }
            }
        }
    }

    private func toggleScan() async {
        if client.isScanning {
            await client.stopScan()
            return
        }
        let threshold = Int16(rssiThreshold)
        if !rssiThreshold.isEmpty && threshold == nil {
            thresholdInvalid = true
            return
        }
        thresholdInvalid = false
        await client.startScan(
            namePrefix: namePrefix,
            serviceUuid: serviceUuid.isEmpty ? nil : serviceUuid,
            rssiThreshold: threshold
        )
    }
}

/// One discovery row: name, RSSI, identity, and advertisement highlights.
struct ScanRow: View {
    let entry: ScanListModel.Entry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(entry.name ?? "(未命名)")
                    .font(.headline)
                Spacer()
                if let rssi = entry.rssi {
                    Text("\(rssi) dBm")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Text(entry.id.summary)
                .font(.caption2)
                .foregroundStyle(.tertiary)
            HStack(spacing: 8) {
                if let company = entry.manufacturerCompanyId {
                    Text(String(format: "厂商 0x%04x", company))
                }
                if let connectable = entry.isConnectable {
                    Text(connectable ? "可连接" : "不可连接")
                }
                Text("\(entry.services.count) 个服务 UUID")
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }
}
