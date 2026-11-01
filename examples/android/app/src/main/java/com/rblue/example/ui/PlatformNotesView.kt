package com.rblue.example.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rblue.example.client.AdapterState
import com.rblue.example.client.CapabilitySet
import com.rblue.example.client.RblueClient

/**
 * Platform facts page: what the Android backend (fork shell over the
 * btleplug Java helpers) can and cannot do, the initialization discipline,
 * and live diagnostics from the bound adapter. Content mirrors
 * crates/rblue-android/docs/platform-notes.md.
 */
@Composable
fun PlatformNotesView(client: RblueClient) {
    val adapterState by client.adapterState.collectAsStateWithLifecycle()
    LazyColumn(
        modifier = Modifier.fillMaxWidth(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            NotesCard("适配器诊断") {
                Text("backendVersion:${client.backendVersion}")
                Text("adapterId:${client.adapterId}")
                Text("适配器状态:${adapterState.localizedDescription()}")
                if (client.capabilities.names.isEmpty()) {
                    Text(
                        "能力集为空(后端不可用)",
                        style = MaterialTheme.typography.bodySmall,
                    )
                } else {
                    Text(
                        client.capabilities.names.sorted().joinToString(" · "),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
        }
        item {
            NotesCard(
                "初始化纪律",
                footer = "蓝牙标识属于个人数据;示例不把设备地址或名称写入应用日志。",
            ) {
                Text("RblueAndroid.initialize(context) 必须在任何 Rblue() 构造之前调用," +
                    "且要在能看到应用 ClassLoader 的应用线程上(主线程最稳):fork 通过" +
                    "调用线程的 ClassLoader 解析 com.nonpolynomial.btleplug 辅助类并注册 " +
                    "JNI native 方法。")
                Text("永远不要在 static 初始化器中调用:那时 Android ClassLoader 尚未就绪。" +
                    "本示例在 Application.onCreate 中调用;失败会抛出带说明的异常" +
                    "(典型原因是 AAR 的 Java 载荷被剥离)。")
                Text("调用幂等,可重复;native 侧 init 由 rblue-android 自行去重。")
            }
        }
        item {
            NotesCard("设备标识") {
                Text("本平台只有 DeviceId::Mac(BluetoothDevice.getAddress()),且地址类型" +
                    "恒为 unknown:Android 不暴露地址种类。")
                Text("RPA(可解析私有地址)约每 15 分钟轮换,轮换后同一设备如同新设备;" +
                    "跨轮换重连要重新扫描并按名称/广播指纹匹配,不要持久化 MAC。")
                Text("断连不会把设备从注册表驱逐(注册表存活于适配器生命周期):" +
                    "断开后可直接对原 DeviceId 再次 connect,无需重新扫描。")
            }
        }
        item {
            NotesCard("GATT 与写语义") {
                Text("服务发现必须在每次连接后执行(Android 栈缓存可能过期);" +
                    "特征标识只在当前连接内有效,重连后必须重新 discover_services。")
                Text("统一写语义:超过 max_write_len 的载荷被 InvalidLength 拒绝——" +
                    "不截断、不分片。原生 Android 栈会静默截断超长写;" +
                    "设备详情页的「超长写入演示」可以看到这一差异。")
                Text("fork 在每次连接自动 requestMtu(517);协商完成前 ATT MTU 为 23," +
                    "即 max_write_len 保守为 20。每次(重)连接后重新查询,不要缓存。")
                Text("通知订阅随连接死亡:重连后必须重新 subscribe,值才会恢复流动" +
                    "(即使设备已绑定,示例也按此纪律执行)。")
                Text("同 UUID 的多个特征会被 fork 的 Java 层去重,实例序号实际恒为 0。")
                Text("断连原因:fork 不上报状态码,DeviceDisconnected 一律 unknown。")
            }
        }
        item {
            NotesCard("GATT status 133") {
                Text("133 是 Android GATT 的通用错误码(GATT_ERROR):连接超时、链路丢失、" +
                    "栈内部状态都报告它,与具体操作几乎无关,是 Android BLE 的头号痛点。")
                Text("fork 的 Java 层对「快速失败」的 133 连接尝试自动重试(共 3 次;" +
                    "Android ≤14 的约 30 s 直连超时也报告 133,重试它会变成约 90 s 等待," +
                    "所以只对快速失败重试)。穷尽后错误面向应用呈现。")
                Text("rblue 把原始码放进 Error::Platform{code} 原样透传;" +
                    "本示例的重连策略与错误文案都匹配原始码,不降级为字符串。")
            }
        }
        item {
            NotesCard(
                "后台与权限",
                footer = "拒绝路径不会崩溃:权限拒绝/蓝牙关闭以结构化 Permission(…) 弹窗呈现。",
            ) {
                Text("API 31+(Android 12+):扫描需要运行时权限 BLUETOOTH_SCAN" +
                    "(本示例声明 neverForLocation,不从不广播推导位置),连接与 GATT 需要 " +
                    "BLUETOOTH_CONNECT。")
                Text("API 30 及以下:扫描需要 ACCESS_FINE_LOCATION,且系统定位服务须开启;" +
                    "连接无需蓝牙运行时权限。")
                Text("蓝牙关闭:Permission(BluetoothOff)。fork 不追踪电台状态、不发 " +
                    "state_changed 事件,initialize 在关蓝牙时也成功;首个真实操作才报告。")
                Text("扫描失败被 fork 丢弃:扫描中途关蓝牙只会让列表不再更新,不会报错。")
                Text("后台扫描需要前台服务(FOREGROUND_SERVICE + " +
                    "FOREGROUND_SERVICE_CONNECTED_DEVICE);后端保守地把 BackgroundScan " +
                    "报告为 false,本示例只做前台扫描。")
            }
        }
        item {
            NotesCard("能力矩阵") {
                CapabilityRow.entries.forEach { row ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 2.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Text(row.displayName, style = MaterialTheme.typography.bodySmall)
                        Text(
                            if (row.supported) "支持" else "不支持",
                            style = MaterialTheme.typography.bodySmall,
                            color = if (row.supported) {
                                MaterialTheme.colorScheme.primary
                            } else {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            },
                        )
                    }
                }
                Text(
                    "与当前适配器 capabilities() 不一致时请上报" +
                        "(勾选对照见上方诊断卡片:${CapabilityRow.entries.count { it.supported }} 项支持)。",
                    style = MaterialTheme.typography.bodySmall,
                )
                // Live cross-check: surfaces any drift between the static
                // matrix and the adapter's runtime capability set.
                val drift = CapabilityRow.entries.filter {
                    it.supported != client.capabilities.has(it.capabilityName)
                }
                if (drift.isNotEmpty()) {
                    Text(
                        "差异项:" + drift.joinToString(" · ") { it.capabilityName },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        }
        item {
            NotesCard("模拟器与验收") {
                Text("Android 模拟器没有蓝牙硬件:real flavor 只能完成链接级验证;" +
                    "运行验收(扫描、连接、读写、通知、断连重连)必须真机。")
                Text("无硬件冒烟用 mock flavor(模拟器/CI 可装):RblueAndroid.initialize " +
                    "为 no-op,后端是 Rust mock 适配器,UI 全流程可走但无真实无线电。")
            }
        }
    }
}

@Composable
private fun NotesCard(title: String, footer: String? = null, content: @Composable () -> Unit) {
    Card {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            content()
            footer?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/** (display name, capability bit name in the JSON set, supported per the
 * platform notes). */
private enum class CapabilityRow(
    val displayName: String,
    val capabilityName: String,
    val supported: Boolean,
) {
    ActiveScan("ActiveScan", "ACTIVE_SCAN", true),
    GattClient("GattClient", "GATT_CLIENT", true),
    MtuQuery("MtuQuery", "MTU_QUERY", true),
    RssiDuringConnection("RssiDuringConnection", "RSSI_DURING_CONNECTION", true),
    MtuNegotiation("MtuNegotiation", "MTU_NEGOTIATION", false),
    RequestDevicePicker("RequestDevicePicker", "REQUEST_DEVICE_PICKER", false),
    Pairing("Pairing", "PAIRING", false),
    BackgroundScan("BackgroundScan", "BACKGROUND_SCAN", false),
    PassiveScan("PassiveScan", "PASSIVE_SCAN", false),
    ScanResponse("ScanResponse", "SCAN_RESPONSE", false),
    GattServer("GattServer", "GATT_SERVER", false),
    Advertise("Advertise", "ADVERTISE", false),
    WatchAdvertisements("WatchAdvertisements", "WATCH_ADVERTISEMENTS", false),
}
