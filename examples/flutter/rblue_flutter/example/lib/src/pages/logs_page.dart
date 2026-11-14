import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../app_state.dart';

/// User-initiated export; snapshots may include device identifiers and payloads.
Future<void> copySession(BuildContext context, AppState state) async {
  await Clipboard.setData(ClipboardData(text: state.sessionJson()));
  if (context.mounted) {
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(const SnackBar(content: Text('会话 JSON 已复制到剪贴板')));
  }
}

/// Bounded timeline of actual adapter and application operations.
class LogsPage extends StatefulWidget {
  const LogsPage({super.key});

  @override
  State<LogsPage> createState() => _LogsPageState();
}

class _LogsPageState extends State<LogsPage> {
  bool errorsOnly = false;

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    final colors = Theme.of(context).colorScheme;
    final entries = state.logs.where((entry) => !errorsOnly || entry.isError);
    return ListView(
      key: const PageStorageKey('logs'),
      padding: const EdgeInsets.all(16),
      children: [
        const Text('最近 300 条事件 · 会话导出包含设备标识和通知内容'),
        const SizedBox(height: 12),
        Align(
          alignment: AlignmentDirectional.centerStart,
          child: FilterChip(
            label: const Text('仅错误'),
            selected: errorsOnly,
            onSelected: (value) => setState(() => errorsOnly = value),
          ),
        ),
        const SizedBox(height: 12),
        if (entries.isEmpty)
          const Padding(padding: EdgeInsets.all(24), child: Text('当前没有匹配的事件。')),
        for (final entry in entries)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Card(
              color: entry.isError ? colors.errorContainer : null,
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '${entry.time.toLocal().toString().substring(11, 23)} · ${entry.isError ? "ERROR" : "INFO"}',
                      style: Theme.of(context).textTheme.labelMedium?.copyWith(
                        color: entry.isError ? colors.onErrorContainer : null,
                      ),
                    ),
                    Text(
                      entry.action,
                      style: Theme.of(context).textTheme.titleMedium?.copyWith(
                        color: entry.isError ? colors.onErrorContainer : null,
                      ),
                    ),
                    const SizedBox(height: 4),
                    SelectableText(
                      entry.detail,
                      style: TextStyle(
                        color: entry.isError
                            ? colors.onErrorContainer
                            : colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }
}
