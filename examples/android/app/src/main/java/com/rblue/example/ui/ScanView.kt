package com.rblue.example.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rblue.example.BlePermissions
import com.rblue.example.client.AdapterState
import com.rblue.example.model.ScanEntry
import com.rblue.example.client.DeviceId
import com.rblue.example.client.RblueClient
import kotlinx.coroutines.launch

/**
 * Scan screen: permission gate, adapter state banner, filter inputs, and the
 * deduplicated discovery list sorted by RSSI.
 */
@Composable
fun ScanView(client: RblueClient, onOpenDevice: (DeviceId) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val adapterState by client.adapterState.collectAsStateWithLifecycle()
    val backendVersion = client.backendVersion
    val scan by client.scan.collectAsStateWithLifecycle()
    val isScanning by client.isScanning.collectAsStateWithLifecycle()

    var namePrefix by rememberSaveable { mutableStateOf("") }
    var serviceUuid by rememberSaveable { mutableStateOf("") }
    var rssiThreshold by rememberSaveable { mutableStateOf("") }
    var thresholdInvalid by rememberSaveable { mutableStateOf(false) }
    var missingPermissions by remember { mutableStateOf(BlePermissions.missing(context)) }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { _ ->
        missingPermissions = BlePermissions.missing(context)
    }

    LazyColumn(
        modifier = Modifier.fillMaxWidth(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            Card {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("适配器", style = MaterialTheme.typography.titleSmall)
                    Text("状态:${adapterState.localizedDescription()}")
                    Text("backendVersion:$backendVersion")
                    if (adapterState != AdapterState.POWERED_ON) {
                        OutlinedButton(onClick = { scope.launch { client.initializeAdapter() } }) {
                            Text("重新初始化")
                        }
                    }
                }
            }
        }
        if (missingPermissions.isNotEmpty()) {
            item {
                Card {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text("需要权限", style = MaterialTheme.typography.titleSmall)
                        Text(
                            "扫描与连接需要" +
                                if (android.os.Build.VERSION.SDK_INT >= 31) {
                                    "「附近的设备」(BLUETOOTH_SCAN / BLUETOOTH_CONNECT)"
                                } else {
                                    "「位置信息」(ACCESS_FINE_LOCATION)"
                                } + "运行时权限;拒绝后操作会以结构化 Permission 错误呈现。",
                            style = MaterialTheme.typography.bodySmall,
                        )
                        Button(onClick = { permissionLauncher.launch(missingPermissions.toTypedArray()) }) {
                            Text("授予蓝牙权限")
                        }
                    }
                }
            }
        }
        item {
            Card {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("过滤(可选)", style = MaterialTheme.typography.titleSmall)
                    OutlinedTextField(
                        value = namePrefix,
                        onValueChange = { namePrefix = it },
                        label = { Text("名称前缀") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = serviceUuid,
                        onValueChange = { serviceUuid = it },
                        label = { Text("服务 UUID(如 180F / 完整 UUID)") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = rssiThreshold,
                        onValueChange = { rssiThreshold = it },
                        label = { Text("RSSI 阈值 dBm(如 -80)") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    if (thresholdInvalid) {
                        Text(
                            "RSSI 阈值必须是整数 dBm",
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.End,
                    ) {
                        Button(onClick = {
                            scope.launch {
                                if (isScanning) {
                                    client.stopScan()
                                    return@launch
                                }
                                if (missingPermissions.isNotEmpty()) {
                                    permissionLauncher.launch(missingPermissions.toTypedArray())
                                    return@launch
                                }
                                val threshold = rssiThreshold.toIntOrNull()
                                if (rssiThreshold.isNotEmpty() && threshold == null) {
                                    thresholdInvalid = true
                                    return@launch
                                }
                                thresholdInvalid = false
                                client.startScan(
                                    namePrefix = namePrefix,
                                    serviceUuid = serviceUuid,
                                    rssiThreshold = threshold,
                                )
                            }
                        }) {
                            Text(if (isScanning) "停止扫描" else "开始扫描")
                        }
                    }
                }
            }
        }
        item {
            Text(
                "设备(${scan.entries.size})" + if (isScanning) " · 扫描中…" else "",
                style = MaterialTheme.typography.titleSmall,
            )
            HorizontalDivider()
        }
        if (scan.entries.isEmpty()) {
            item {
                Text(
                    if (isScanning) "扫描中…(真机对周边外设广播)" else "点上方「开始扫描」",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        items(scan.sortedByRssi, key = { it.id.summary }) { entry ->
            ScanRow(entry = entry, onClick = { onOpenDevice(entry.id) })
        }
    }
}

/** One discovery row: name, RSSI, identity, and advertisement highlights. */
@Composable
private fun ScanRow(entry: ScanEntry, onClick: () -> Unit) {
    Card(onClick = onClick) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    entry.name ?: "(未命名)",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                entry.rssi?.let { Text("$it dBm", style = MaterialTheme.typography.labelSmall) }
            }
            Text(
                entry.id.summary,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                entry.manufacturerCompanyId?.let {
                    Text("厂商 0x%04x".format(it), style = MaterialTheme.typography.labelSmall)
                }
                entry.isConnectable?.let {
                    Text(if (it) "可连接" else "不可连接", style = MaterialTheme.typography.labelSmall)
                }
                Text("${entry.services.size} 个服务 UUID", style = MaterialTheme.typography.labelSmall)
            }
        }
    }
}
