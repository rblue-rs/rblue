package com.rblue.example.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rblue.example.client.DeviceId
import com.rblue.example.client.Notification
import com.rblue.example.client.RblueClient
import com.rblue.example.model.DataRendering
import com.rblue.example.model.DeviceDetailModel
import kotlinx.coroutines.launch

/**
 * Device detail: connection header with the reconnect banner, the
 * platform-limitation probe, and the discovered service/characteristic tree
 * with read/write/subscribe actions.
 */
@Composable
fun DeviceDetailView(client: RblueClient, device: DeviceId, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    val model = remember(device) { DeviceDetailModel(device, client) }

    // Discovery is mandatory on every connection; a successful reconnect
    // (isConnected flips) re-discovers and re-issues kept subscriptions.
    LaunchedEffect(model) { model.discover() }
    val connected = client.isConnected(device)
    LaunchedEffect(connected) { if (connected) model.discover() }
    // View teardown: cancel remaining subscriptions like the iOS example.
    DisposableEffect(model) {
        onDispose { client.launchInBackground { model.unsubscribeAll() } }
    }

    val services by model.services.collectAsStateWithLifecycle()
    val rssi by model.rssi.collectAsStateWithLifecycle()
    val mtu by model.mtu.collectAsStateWithLifecycle()
    val deviceName by model.deviceName.collectAsStateWithLifecycle()
    val isDiscovering by model.isDiscovering.collectAsStateWithLifecycle()
    val busyIds by model.busyCharacteristicIds.collectAsStateWithLifecycle()
    val modelError by model.presentedError.collectAsStateWithLifecycle()
    val connection = client.connections.collectAsStateWithLifecycle().value[device]
    val notifications = client.notifications(device)

    LazyColumn(
        modifier = Modifier.fillMaxWidth(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回")
                }
                Text(
                    deviceName ?: "设备",
                    style = MaterialTheme.typography.titleLarge,
                )
            }
        }
        item {
            Card {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("连接", style = MaterialTheme.typography.titleSmall)
                    Text("名称:${deviceName ?: "(未知)"}")
                    Text("标识:${device.summary}")
                    Text("状态:${if (connected) "已连接" else "未连接"}")
                    rssi?.let { Text("RSSI:$it dBm") }
                    mtu?.let { Text("MTU:$it") }
                    connection?.reconnect?.let { reconnect ->
                        Column {
                            Text(
                                "自动重连:第 ${reconnect.attempt} 次尝试,约 " +
                                    "${reconnect.nextDelayMillis} ms 后",
                                color = MaterialTheme.colorScheme.tertiary,
                            )
                            reconnect.lastError?.let {
                                Text(it, style = MaterialTheme.typography.bodySmall)
                            }
                        }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = {
                            scope.launch {
                                if (connected) client.disconnect(device) else client.connect(device)
                            }
                        }) {
                            Text(if (connected) "断开连接" else "连接")
                        }
                        if (connected) {
                            OutlinedButton(
                                onClick = { scope.launch { model.probePair() } },
                            ) { Text("pair() 探针") }
                        }
                    }
                    if (isDiscovering) {
                        Text("服务发现:进行中…", style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
        }
        if (connected) {
            item {
                Text(
                    "服务与特征(${services.size} 个服务)",
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    "特征标识是会话级的:重连后必须重新发现服务。「超长写入」演示统一写语义 - " +
                        "rblue 以 InvalidLength 拒绝,而原生 Android 栈会静默截断超长写。",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            items(services, key = { it.id }) { service ->
                ServiceCard(
                    model = model,
                    service = service,
                    busyIds = busyIds,
                    notifications = notifications,
                )
            }
        }
    }

    modelError?.let { error ->
        ErrorAlert(error = error, onDismiss = model::dismissError)
    }
}

/** One service with its characteristics (collapsed by default keeps long
 * trees navigable). */
@Composable
private fun ServiceCard(
    model: DeviceDetailModel,
    service: DeviceDetailModel.ServiceNode,
    busyIds: Set<String>,
    notifications: List<Notification>,
) {
    var expanded by rememberSaveable(service.id) { mutableStateOf(false) }
    Card {
        Column(Modifier.padding(12.dp)) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { expanded = !expanded },
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Column {
                    Text(service.uuid, style = MaterialTheme.typography.titleSmall)
                    Text(
                        (if (service.primary) "primary" else "secondary") +
                            " · ${service.characteristics.size} 个特征",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(if (expanded) "收起" else "展开", style = MaterialTheme.typography.labelSmall)
            }
            if (expanded) {
                service.characteristics.forEach { node ->
                    HorizontalDivider(Modifier.padding(vertical = 8.dp))
                    CharacteristicSection(
                        model = model,
                        service = service,
                        node = node,
                        busy = node.id in busyIds,
                        notifications = notifications,
                    )
                }
            }
        }
    }
}

/** One characteristic with its actions: read, write (both modes), the
 * oversize-write demonstration, and the notification log. */
@Composable
private fun CharacteristicSection(
    model: DeviceDetailModel,
    service: DeviceDetailModel.ServiceNode,
    node: DeviceDetailModel.CharacteristicNode,
    busy: Boolean,
    notifications: List<Notification>,
) {
    val scope = rememberCoroutineScope()
    var writeText by rememberSaveable(node.id) { mutableStateOf("") }
    var withResponse by rememberSaveable(node.id) { mutableStateOf(true) }

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            Text(node.uuid, style = MaterialTheme.typography.bodySmall)
            Text(
                "  " + node.propertyBadges.joinToString(" "),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (node.subscribed) {
                Text(
                    "  已订阅",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
        }
        node.value?.let { value ->
            Text("当前值:${value.size} 字节", style = MaterialTheme.typography.bodySmall)
            Text(
                DataRendering.hex(value),
                style = MaterialTheme.typography.bodySmall,
            )
            Text(
                DataRendering.text(value),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } ?: Text("当前值:尚未读取", style = MaterialTheme.typography.bodySmall)
        node.lastAction?.let {
            Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (node.canRead) {
            OutlinedButton(
                enabled = !busy,
                onClick = { scope.launch { model.read(node) } },
            ) { Text("读取") }
        }
        if (node.canWrite || node.canWriteWithoutResponse) {
            OutlinedTextField(
                value = writeText,
                onValueChange = { writeText = it },
                label = { Text("写入文本载荷") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                SegmentedButton(
                    selected = withResponse,
                    onClick = { withResponse = true },
                    shape = SegmentedButtonDefaults.itemShape(index = 0, count = 2),
                ) { Text("With response") }
                SegmentedButton(
                    selected = !withResponse,
                    onClick = { withResponse = false },
                    shape = SegmentedButtonDefaults.itemShape(index = 1, count = 2),
                ) { Text("Without response") }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    enabled = !busy && writeText.isNotEmpty(),
                    onClick = {
                        val payload = writeText.toByteArray(Charsets.UTF_8)
                        scope.launch { model.write(node, payload, withResponse) }
                    },
                ) { Text("写入") }
                OutlinedButton(
                    enabled = !busy,
                    onClick = { scope.launch { model.writeOversizeDemo(node, withResponse) } },
                ) { Text("超长写入演示") }
            }
        }
        if (node.canNotify || node.canIndicate) {
            OutlinedButton(
                enabled = !busy,
                onClick = { scope.launch { model.toggleSubscription(node) } },
            ) { Text(if (node.subscribed) "取消订阅" else "订阅") }
            if (node.subscribed) {
                val recent = notifications
                    .filter { it.id.uuid == node.uuid && it.id.service.uuid == service.uuid }
                    .takeLast(5)
                    .reversed()
                Text(
                    "通知(最近 ${recent.size} 条,共 ${notifications.count {
                        it.id.uuid == node.uuid && it.id.service.uuid == service.uuid
                    }} 条)",
                    style = MaterialTheme.typography.labelSmall,
                )
                recent.forEach { notification ->
                    Column {
                        Text(
                            "${notification.data.size} 字节 · ${DataRendering.hex(notification.data)}",
                            style = MaterialTheme.typography.labelSmall,
                        )
                        Text(
                            DataRendering.text(notification.data),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}
