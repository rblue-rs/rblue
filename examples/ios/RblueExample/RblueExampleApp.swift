import SwiftUI

@main
struct RblueExampleApp: App {
    @StateObject private var client = RblueClient()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(client)
        }
    }
}
