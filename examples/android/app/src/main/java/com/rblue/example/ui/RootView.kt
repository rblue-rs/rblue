package com.rblue.example.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.rblue.example.client.AdapterState
import com.rblue.example.client.DeviceId
import com.rblue.example.client.PresentedError
import com.rblue.example.client.RblueClient

/** Material 3 theme; the Activity-level XML theme only bootstraps the
 * window, all styling happens here. */
@Composable
fun RblueTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme(),
        content = content,
    )
}

/** Root tab layout (scan / platform notes) plus the shared alert surface for
 * client-level errors, and the scan -> device-detail navigation. */
@Composable
fun RootView(client: RblueClient) {
    val presentedError by client.presentedError.collectAsStateWithLifecycle()
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var selectedDevice by remember { mutableStateOf<DeviceId?>(null) }

    LaunchedEffect(Unit) {
        // The sink must be installed before initialize so adapter state
        // events are not missed.
        client.start()
        client.initializeAdapter()
    }

    BackHandler(enabled = selectedDevice != null) {
        selectedDevice = null
    }

    RblueTheme {
        Scaffold(
            bottomBar = {
                if (selectedDevice == null) {
                    NavigationBar {
                        NavigationBarItem(
                            selected = tab == 0,
                            onClick = { tab = 0 },
                            icon = { Icon(Icons.Filled.Search, contentDescription = null) },
                            label = { Text("扫描") },
                        )
                        NavigationBarItem(
                            selected = tab == 1,
                            onClick = { tab = 1 },
                            icon = { Icon(Icons.Filled.Info, contentDescription = null) },
                            label = { Text("平台说明") },
                        )
                    }
                }
            },
        ) { padding ->
            Box(Modifier.fillMaxSize().padding(padding)) {
                val device = selectedDevice
                when {
                    device != null -> DeviceDetailView(
                        client = client,
                        device = device,
                        onBack = { selectedDevice = null },
                    )
                    tab == 0 -> ScanView(
                        client = client,
                        onOpenDevice = { selectedDevice = it },
                    )
                    else -> PlatformNotesView(client)
                }
            }
        }
    }

    presentedError?.let { error ->
        ErrorAlert(error = error, onDismiss = client::dismissError)
    }
}

/** Shared alert rendering for structured errors (client- and model-level):
 * detail plus the verbatim structured form, matching the facade's error
 * contract. */
@Composable
fun ErrorAlert(error: PresentedError, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(error.title) },
        text = { Text("${error.detail}\n\n${error.structured}") },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("好") }
        },
    )
}

/** User-facing adapter state text. */
fun AdapterState.localizedDescription(): String = when (this) {
    AdapterState.POWERED_ON -> "已开启"
    AdapterState.POWERED_OFF -> "已关闭"
    AdapterState.UNAUTHORIZED -> "未授权(系统设置 > 应用 > 权限)"
    AdapterState.UNAVAILABLE -> "不可用"
    AdapterState.UNKNOWN -> "未知"
}
