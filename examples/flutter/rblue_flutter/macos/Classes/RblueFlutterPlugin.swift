import Cocoa
import FlutterMacOS

/// macOS side of the plugin.
///
/// CoreBluetooth needs no host-side initialization. The channel keeps the Dart
/// startup path identical across Apple platforms.
public class RblueFlutterPlugin: NSObject, FlutterPlugin {
    public static func register(with registrar: FlutterPluginRegistrar) {
        let channel = FlutterMethodChannel(
            name: "rblue_flutter",
            binaryMessenger: registrar.messenger
        )
        let instance = RblueFlutterPlugin()
        registrar.addMethodCallDelegate(instance, channel: channel)
    }

    public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
        switch call.method {
        case "initialize":
            result(true)
        case "requestBlePermissions":
            // macOS prompts on first CoreBluetooth use. The host application is
            // responsible for its Bluetooth usage description and entitlements.
            result(true)
        default:
            result(FlutterMethodNotImplemented)
        }
    }
}
