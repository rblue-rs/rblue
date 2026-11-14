import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import '../payload.dart';
import '../widgets/expressive_button.dart';
import 'scan_page.dart' show MaterialBannerTile;

/// Stateful inspector kept below the application's persistent navigation bar.
class DeviceDetailPage extends StatefulWidget {
  const DeviceDetailPage({
    super.key,
    required this.state,
    required this.device,
  });

  final AppState state;
  final DiscoveredDevice device;

  @override
  State<DeviceDetailPage> createState() => _DeviceDetailPageState();
}

class _DeviceDetailPageState extends State<DeviceDetailPage> {
  final _payload = TextEditingController(text: '48 65 6C 6C 6F');
  StreamSubscription<AdapterEvent>? _events;
  ServiceSet? _services;
  (CharacteristicId, Characteristic)? _selected;
  bool _busy = false;
  bool _hex = true;
  int _tab = 0;
  int _revision = 0;
  int? _maximum;
  int _limitRequest = 0;
  WriteMode _mode = WriteMode.withResponse;
  String? _error;
  String _result = '选择特征后执行操作。';
  int? _rssi;
  int? _mtu;

  AppState get state => widget.state;
  RblueClient get client => state.client!;
  DeviceId get device => widget.device.id;

  @override
  void initState() {
    super.initState();
    _events = client.events.listen((event) {
      if (event is AdapterDeviceDisconnected && event.id == device ||
          event is AdapterStateChanged &&
              event.state != AdapterState.poweredOn) {
        if (!mounted) return;
        setState(() {
          _revision++;
          _services = null;
          _selected = null;
          _maximum = null;
          _rssi = null;
          _mtu = null;
          _result = '连接已失效，请重新连接并发现服务。';
        });
      }
    });
    if (state.isConnected(device)) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _run(_discover);
      });
    }
  }

  @override
  void dispose() {
    _events?.cancel();
    _payload.dispose();
    super.dispose();
  }

  Future<void> _run(Future<void> Function() action) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await action();
    } catch (error) {
      if (mounted) {
        setState(
          () => _error = error is RblueException
              ? '${error.kind.name}: ${error.message}${error.isAndroidGattStatus133 ? " (GATT 133)" : ""}'
              : error is FormatException
              ? error.message
              : error.toString(),
        );
      }
      state.record('operation.error', '$error', isError: true);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _connect() async {
    final revision = _revision;
    await state.operation(
      'connect / ${device.display}',
      () => client.connect(device),
    );
    if (!mounted || revision != _revision) return;
    state.connected.add(state.deviceKey(device));
    state.record('connect.ready', device.display);
    await _discover();
  }

  Future<void> _disconnect() async {
    await state.operation(
      'disconnect / ${device.display}',
      () => client.disconnect(device),
    );
    state.connected.remove(state.deviceKey(device));
    state.invalidateSubscriptions(device);
    if (!mounted) return;
    setState(() {
      _revision++;
      _services = null;
      _selected = null;
      _maximum = null;
      _rssi = null;
      _mtu = null;
    });
  }

  Future<void> _discover() async {
    await state.cancelDeviceSubscriptions(device);
    if (!mounted || !state.isConnected(device)) return;
    state.invalidateSubscriptions(device);
    final revision = _revision;
    final services = await state.operation(
      'discover / ${device.display}',
      () => client.discoverServices(device),
    );
    if (!mounted || revision != _revision || !state.isConnected(device)) return;
    final attributes = services.identifiedCharacteristics.toList();
    setState(() {
      _services = services;
      _selected = null;
    });
    if (attributes.isNotEmpty) _select(attributes.first);
  }

  void _select((CharacteristicId, Characteristic) value) {
    setState(() {
      _selected = value;
      _result = '选择特征后执行操作。';
      _maximum = null;
      _error = null;
      _mode = value.$2.writable
          ? WriteMode.withResponse
          : WriteMode.withoutResponse;
    });
    _loadMaximum();
  }

  Future<void> _loadMaximum() async {
    final request = ++_limitRequest;
    final selected = _selected;
    if (selected == null ||
        (!selected.$2.writable && !selected.$2.writableWithoutResponse)) {
      return;
    }
    final revision = _revision;
    try {
      final maximum = await client.maxWriteLength(
        device,
        selected.$1,
        mode: _mode,
      );
      if (mounted && revision == _revision && request == _limitRequest) {
        setState(() => _maximum = maximum);
      }
    } catch (error) {
      state.record('max_write_len', '$error', isError: true);
    }
  }

  Future<void> _read() async {
    final id = _selected!.$1;
    final revision = _revision;
    final data = await state.operation(
      'read / ${id.uuid} #${id.instance}',
      () => client.read(device, id),
    );
    if (!mounted || revision != _revision) return;
    state.record(
      'read.result / ${device.display} / ${id.service.uuid} #${id.service.instance} / ${id.uuid} #${id.instance}',
      _format(data),
    );
    final hex =
        '${data.length} B · HEX: ${data.isEmpty ? "（空）" : _format(data)}';
    try {
      final text = utf8.decode(data);
      final printable = text.runes.every(
        (rune) =>
            rune == 9 ||
            rune == 10 ||
            rune == 13 ||
            (rune >= 32 && (rune < 127 || rune > 159)),
      );
      _result = text.isNotEmpty && printable ? '$hex\nUTF-8: $text' : hex;
    } on FormatException {
      _result = hex;
    }
  }

  Future<void> _write({bool oversize = false}) async {
    final selected = _selected!;
    final revision = _revision;
    final data = oversize
        ? List.filled((_maximum ?? 599) + 1, 0x41)
        : parsePayload(_payload.text, hex: _hex);
    if (data.isEmpty) throw const FormatException('请输入发送内容。');
    if (!oversize && _maximum != null && data.length > _maximum!) {
      throw FormatException(
        'InvalidLength: ${data.length} bytes > $_maximum bytes，不会截断或自动分包。',
      );
    }
    await state.operation(
      'write / ${selected.$1.uuid} #${selected.$1.instance} / ${_mode.name} / ${data.length} B',
      () => client.write(device, selected.$1, data, mode: _mode),
    );
    if (!mounted || revision != _revision) return;
    _result = oversize
        ? '超长写入被后端接受，请核对平台限制。'
        : '写入成功 · ${data.length} bytes · ${_mode.name}';
  }

  String _format(List<int> data) =>
      data.map((value) => value.toRadixString(16).padLeft(2, '0')).join(' ');

  @override
  Widget build(BuildContext context) {
    AppStateScope.of(context);
    final colors = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final linked = state.isConnected(device);
    final systemManagedMtu = Theme.of(context).platform == TargetPlatform.iOS;
    final characteristic = _selected?.$2;
    final writable =
        characteristic != null &&
        (characteristic.writable || characteristic.writableWithoutResponse);
    final bytes = switch (_tryPayload()) {
      final int n => '$n bytes',
      _ => '格式待修正',
    };
    return DefaultTabController(
      length: 3,
      child: ListView(
        key: PageStorageKey('inspector-${device.display}'),
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
        keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
        children: [
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(widget.device.displayName, style: text.headlineSmall),
                  const SizedBox(height: 4),
                  SelectableText(device.display, style: text.bodySmall),
                  const SizedBox(height: 12),
                  Text(
                    linked ? '已连接 · 每次连接重新发现服务' : '未连接',
                    style: text.labelMedium,
                  ),
                  const SizedBox(height: 16),
                  SquishButtonGroup(
                    actions: [
                      (
                        label: linked ? '断开连接' : '连接设备',
                        tonal: false,
                        onPressed:
                            _busy ||
                                widget.device.advertisement.isConnectable ==
                                    false
                            ? null
                            : () => _run(linked ? _disconnect : _connect),
                      ),
                      (
                        label: '重发现',
                        tonal: true,
                        onPressed: linked && !_busy
                            ? () => _run(_discover)
                            : null,
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
          if (_busy)
            const Padding(
              padding: EdgeInsets.symmetric(vertical: 12),
              child: LinearProgressIndicator(),
            ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: MaterialBannerTile(
                icon: Icons.error_outline,
                text: _error!,
              ),
            ),
          const SizedBox(height: 12),
          TabBar(
            onTap: (value) => setState(() => _tab = value),
            tabs: const [
              Tab(text: 'GATT'),
              Tab(text: '广播数据'),
              Tab(text: '连接信息'),
            ],
          ),
          const SizedBox(height: 16),
          if (_tab == 0) ...[
            if (_services == null)
              const Padding(
                padding: EdgeInsets.all(24),
                child: Text('连接设备后浏览 GATT 服务与特征。'),
              ),
            if (_services != null) ...[
              Text(
                '${_services!.services.length} 个服务 · ${_services!.allCharacteristics.length} 个特征',
                style: text.bodySmall,
              ),
              const SizedBox(height: 12),
              for (final (serviceIndex, service) in _services!.services.indexed)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Card(
                    child: Padding(
                      padding: const EdgeInsets.all(16),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            '${service.primary ? "Primary" : "Secondary"} service',
                            style: text.labelSmall,
                          ),
                          SelectableText(service.uuid, style: text.titleSmall),
                          const SizedBox(height: 8),
                          for (final entry
                              in _services!.identifiedCharacteristics.where(
                                (entry) =>
                                    entry.$1.service ==
                                    ServiceId(
                                      uuid: service.uuid,
                                      instance: _services!.services
                                          .take(serviceIndex)
                                          .where(
                                            (previous) =>
                                                previous.uuid == service.uuid,
                                          )
                                          .length,
                                    ),
                              ))
                            Padding(
                              padding: const EdgeInsets.symmetric(vertical: 3),
                              child: ListTile(
                                shape: RoundedRectangleBorder(
                                  borderRadius: BorderRadius.circular(12),
                                ),
                                tileColor: _selected?.$1 == entry.$1
                                    ? colors.primaryContainer
                                    : colors.surfaceContainerLowest,
                                selected: _selected?.$1 == entry.$1,
                                title: Text(
                                  entry.$1.uuid,
                                  style: text.bodyMedium,
                                ),
                                subtitle: Text(
                                  'service #${entry.$1.service.instance} / characteristic #${entry.$1.instance}\n${[if (entry.$2.readable) 'READ', if (entry.$2.writable) 'WRITE', if (entry.$2.writableWithoutResponse) 'WRITE NR', if (entry.$2.notifiable) 'NOTIFY'].join(' · ')}',
                                  style: text.bodySmall,
                                ),
                                onTap: _busy ? null : () => _select(entry),
                              ),
                            ),
                        ],
                      ),
                    ),
                  ),
                ),
            ],
            if (_selected != null && linked)
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text('特征操作', style: text.titleMedium),
                      const SizedBox(height: 8),
                      SelectableText(_selected!.$1.uuid, style: text.bodySmall),
                      if (writable) ...[
                        const SizedBox(height: 16),
                        DropdownButtonFormField<bool>(
                          initialValue: _hex,
                          decoration: const InputDecoration(labelText: '数据编码'),
                          items: const [
                            DropdownMenuItem(
                              value: true,
                              child: Text('HEX · 十六进制'),
                            ),
                            DropdownMenuItem(
                              value: false,
                              child: Text('UTF-8 · 文本'),
                            ),
                          ],
                          onChanged: _busy
                              ? null
                              : (value) => setState(() => _hex = value!),
                        ),
                        const SizedBox(height: 12),
                        DropdownButtonFormField<WriteMode>(
                          key: ValueKey((
                            _selected!.$1,
                            characteristic.properties,
                          )),
                          initialValue: _mode,
                          decoration: const InputDecoration(labelText: '写入方式'),
                          items: [
                            if (characteristic.writable)
                              const DropdownMenuItem(
                                value: WriteMode.withResponse,
                                child: Text('With response'),
                              ),
                            if (characteristic.writableWithoutResponse)
                              const DropdownMenuItem(
                                value: WriteMode.withoutResponse,
                                child: Text('Without response'),
                              ),
                          ],
                          onChanged: _busy
                              ? null
                              : (value) {
                                  setState(() {
                                    _mode = value!;
                                    _maximum = null;
                                  });
                                  _loadMaximum();
                                },
                        ),
                        const SizedBox(height: 12),
                        TextField(
                          key: const Key('write-payload'),
                          controller: _payload,
                          enabled: !_busy,
                          minLines: 3,
                          maxLines: 6,
                          onChanged: (_) => setState(() {}),
                          decoration: InputDecoration(
                            labelText: '发送内容',
                            helperText: bytes,
                          ),
                        ),
                        const SizedBox(height: 8),
                        Text(
                          _maximum == null
                              ? '最大写长尚未提供，写入由后端验证。'
                              : '最大可写 $_maximum bytes · 当前写入方式',
                          style: text.bodySmall,
                        ),
                        TextButton(
                          onPressed: _busy
                              ? null
                              : () => _run(() => _write(oversize: true)),
                          child: const Text('超长写入验证（预期 InvalidLength）'),
                        ),
                      ],
                      const SizedBox(height: 12),
                      SquishButtonGroup(
                        actions: [
                          if (writable)
                            (
                              label: '写入数据',
                              tonal: false,
                              onPressed: _busy ? null : () => _run(_write),
                            ),
                          if (characteristic!.readable)
                            (
                              label: '读取',
                              tonal: writable,
                              onPressed: _busy ? null : () => _run(_read),
                            ),
                          if (characteristic.notifiable)
                            (
                              label:
                                  state.subscriptions.contains((
                                    device,
                                    _selected!.$1,
                                  ))
                                  ? '取消订阅'
                                  : '订阅通知',
                              tonal: writable || characteristic.readable,
                              onPressed: _busy
                                  ? null
                                  : () => _run(() async {
                                      final id = _selected!.$1;
                                      final enabled = !state.subscriptions
                                          .contains((device, id));
                                      await state.setSubscription(
                                        device,
                                        id,
                                        enabled,
                                      );
                                      _result = enabled
                                          ? '已订阅，前往数据流查看通知。'
                                          : '已取消订阅。';
                                    }),
                            ),
                        ],
                      ),
                      const SizedBox(height: 12),
                      SelectableText(_result, style: text.bodyMedium),
                    ],
                  ),
                ),
              ),
          ],
          if (_tab == 1)
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: SelectableText(
                  const JsonEncoder.withIndent('  ').convert({
                    'identity': device.json,
                    'local_name': widget.device.advertisement.localName,
                    'services': widget.device.advertisement.services,
                    'rssi': state.rssiOf(device),
                    'connectable': widget.device.advertisement.isConnectable,
                    'manufacturer_data':
                        widget.device.advertisement.manufacturerData == null
                        ? null
                        : [
                            widget.device.advertisement.manufacturerData!.$1,
                            widget.device.advertisement.manufacturerData!.$2,
                          ],
                    'service_data': [
                      for (final entry
                          in widget.device.advertisement.serviceData)
                        [entry.$1, entry.$2],
                    ],
                    'tx_power': widget.device.advertisement.txPower,
                    'flags': widget.device.advertisement.flags,
                  }),
                ),
              ),
            ),
          if (_tab == 2)
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      'RSSI: ${_rssi ?? "—"} dBm\nATT MTU: ${systemManagedMtu ? "由系统管理" : "${_mtu ?? "—"} bytes"}',
                    ),
                    const SizedBox(height: 12),
                    SquishButtonGroup(
                      actions: [
                        (
                          label: '读取 RSSI',
                          tonal: true,
                          onPressed: !linked || _busy
                              ? null
                              : () => _run(() async {
                                  final value = await state.operation(
                                    'read_rssi',
                                    () => client.readRssi(device),
                                  );
                                  if (mounted) _rssi = value;
                                }),
                        ),
                        if (!systemManagedMtu)
                          (
                            label: '查询 MTU',
                            tonal: true,
                            onPressed: !linked || _busy
                                ? null
                                : () => _run(() async {
                                    final value = await state.operation(
                                      'query_mtu',
                                      () => client.mtu(device),
                                    );
                                    if (mounted) _mtu = value;
                                  }),
                          ),
                      ],
                    ),
                    const SizedBox(height: 12),
                    const Text('主动 MTU 协商、配对及描述符读写尚未通过 C ABI 暴露。'),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }

  int? _tryPayload() {
    try {
      return parsePayload(_payload.text, hex: _hex).length;
    } on FormatException {
      return null;
    }
  }
}
