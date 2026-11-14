import 'dart:convert';

/// Parses user text without treating UTF-16 code units as Bluetooth bytes.
List<int> parsePayload(String text, {required bool hex}) {
  if (!hex) return utf8.encode(text);
  final value = text.replaceAll(RegExp(r'\s'), '');
  if (value.length.isOdd || !RegExp(r'^[0-9a-fA-F]*$').hasMatch(value)) {
    throw const FormatException('HEX 需要完整字节，例如 48 65 6C 6C 6F');
  }
  return [
    for (var i = 0; i < value.length; i += 2)
      int.parse(value.substring(i, i + 2), radix: 16),
  ];
}
