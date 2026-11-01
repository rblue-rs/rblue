import Flutter
import UIKit

/// iOS side of the example plugin.
///
/// There is no iOS initialization step (the CoreBluetooth backend needs no
/// JVM-side init), so `initialize` simply succeeds; the channel exists so the
/// Dart client can share one startup path across platforms.
public class RblueFlutterPlugin: NSObject, FlutterPlugin {
    public static func register(with registrar: FlutterPluginRegistrar) {
        let channel = FlutterMethodChannel(name: "rblue_flutter", binaryMessenger: registrar.messenger())
        let instance = RblueFlutterPlugin()
        registrar.addMethodCallDelegate(instance, channel: channel)
    }

    public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
        switch call.method {
        case "initialize":
            result(true)
        case "requestBlePermissions":
            // iOS prompts on the first CoreBluetooth use; nothing to request
            // ahead of time beyond the Info.plist usage description.
            result(true)
        default:
            result(FlutterMethodNotImplemented)
        }
    }
}
