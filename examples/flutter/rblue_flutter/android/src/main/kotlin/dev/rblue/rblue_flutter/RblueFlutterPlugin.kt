package dev.rblue.rblue_flutter

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.rblue.RblueNative
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.PluginRegistry

/**
 * Plugin entry point: owns the `rblue_flutter` method channel and performs
 * the Android JNI initialization the rblue-android backend requires.
 *
 * The JNI symbol `Java_com_rblue_RblueNative_nativeInit` is exported by
 * `librblue_c.so` itself (the rblue-android crate is linked in). It must run
 * on a Java thread that can see the application ClassLoader before the first
 * adapter call, so we invoke it here (engine attach, main thread) and expose
 * it to Dart through the `initialize` method as well: Dart may start before
 * [onAttachedToEngine] runs, and the Dart client awaits `initialize` before
 * the first FFI call. The backend init is idempotent, so both paths are safe.
 */
class RblueFlutterPlugin : FlutterPlugin, MethodChannel.MethodCallHandler, ActivityAware {
    private var channel: MethodChannel? = null
    private var activity: Activity? = null
    private var pendingPermissionResult: MethodChannel.Result? = null

    private val permissionListener =
        PluginRegistry.RequestPermissionsResultListener { requestCode, _, grantResults ->
            if (requestCode != REQUEST_CODE) {
                false
            } else {
                completePendingPermission(
                    grantResults.isNotEmpty() &&
                        grantResults.all { it == PackageManager.PERMISSION_GRANTED },
                )
                true
            }
        }

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel = MethodChannel(binding.binaryMessenger, CHANNEL).also {
            it.setMethodCallHandler(this)
        }
        runNativeInit()
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        completePendingPermission(false)
        channel?.setMethodCallHandler(null)
        channel = null
    }

    override fun onAttachedToActivity(binding: ActivityPluginBinding) {
        activity = binding.activity
        binding.addRequestPermissionsResultListener(permissionListener)
    }

    override fun onDetachedFromActivityForConfigChanges() {
        completePendingPermission(false)
        activity = null
    }

    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) {
        activity = binding.activity
        binding.addRequestPermissionsResultListener(permissionListener)
    }

    override fun onDetachedFromActivity() {
        completePendingPermission(false)
        activity = null
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "initialize" -> result.success(runNativeInit())
            "requestBlePermissions" -> requestBlePermissions(result)
            else -> result.notImplemented()
        }
    }

    private fun runNativeInit(): Boolean {
        val ok = RblueNative.nativeInit()
        if (!ok) {
            Log.e(TAG, "RblueNative.nativeInit() failed; adapter calls will not work")
        }
        return ok
    }

    private fun requestBlePermissions(result: MethodChannel.Result) {
        if (pendingPermissionResult != null) {
            result.error(
                "permission_request_in_progress",
                "A Bluetooth permission request is already in progress.",
                null,
            )
            return
        }
        val activity = activity
        if (activity == null) {
            result.success(false)
            return
        }
        val wanted = buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                add(Manifest.permission.BLUETOOTH_SCAN)
                add(Manifest.permission.BLUETOOTH_CONNECT)
            } else {
                add(Manifest.permission.ACCESS_FINE_LOCATION)
            }
        }
        val missing = wanted.filterNot { permission ->
            ContextCompat.checkSelfPermission(activity, permission) ==
                PackageManager.PERMISSION_GRANTED
        }
        if (missing.isEmpty()) {
            result.success(true)
            return
        }
        pendingPermissionResult = result
        ActivityCompat.requestPermissions(activity, missing.toTypedArray(), REQUEST_CODE)
    }

    private fun completePendingPermission(granted: Boolean) {
        pendingPermissionResult?.success(granted)
        pendingPermissionResult = null
    }

    companion object {
        private const val TAG = "RblueFlutterPlugin"
        private const val CHANNEL = "rblue_flutter"
        private const val REQUEST_CODE = 424242
    }
}
