import SwiftUI

/// Root tab layout plus the shared alert surface for client-level errors.
struct RootView: View {
    @EnvironmentObject private var client: RblueClient

    var body: some View {
        TabView {
            ScanView()
                .tabItem { Label("扫描", systemImage: "dot.radiowaves.left.and.right") }
            PlatformNotesView()
                .tabItem { Label("平台说明", systemImage: "info.circle") }
        }
        .task {
            // The sink must be installed before initialize so no adapter
            // state event is missed.
            client.start()
            await client.initializeAdapter()
            if RblueClient.demoModeEnabled {
                await client.startScan(
                    namePrefix: nil,
                    serviceUuid: nil,
                    rssiThreshold: nil
                )
            }
        }
        .alert(
            client.presentedError?.title ?? "",
            isPresented: Binding(
                get: { client.presentedError != nil },
                set: { if !$0 { client.dismissError() } }
            ),
            presenting: client.presentedError
        ) { _ in
            Button("好", role: .cancel) {}
        } message: { error in
            Text("\(error.detail)\n\n\(error.structured)")
        }
    }
}

/// User-facing adapter state text.
extension AdapterState {
    var localizedDescription: String {
        switch self {
        case .poweredOn: return "已开启"
        case .poweredOff: return "已关闭"
        case .unauthorized: return "未授权(系统设置 > 隐私与安全性 > 蓝牙)"
        case .unavailable: return "不可用"
        case .unknown: return "未知"
        }
    }
}

/// Shared alert rendering for model-level errors (device detail screen).
struct ModelErrorAlert: ViewModifier {
    @Binding var model: DeviceDetailModel?

    func body(content: Content) -> some View {
        content.alert(
            model?.presentedError?.title ?? "",
            isPresented: Binding(
                get: { model?.presentedError != nil },
                set: { if !$0 { model?.dismissError() } }
            ),
            presenting: model?.presentedError
        ) { _ in
            Button("好", role: .cancel) {}
        } message: { error in
            Text("\(error.detail)\n\n\(error.structured)")
        }
    }
}
