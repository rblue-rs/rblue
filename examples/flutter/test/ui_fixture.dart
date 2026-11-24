import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:rblue_flutter/rblue_flutter.dart';
import 'package:rblue_flutter/src/ffi_bindings.dart' show RblueHandle;
import 'package:rblue_flutter_example/src/app_state.dart';

import '../rblue_flutter/test/fake_bridge.dart';

final testDeviceId = DeviceId.uuid('C0EA7291-4C8A-40F2-A8F4-001000000001');
const testService = '0000fff0-0000-1000-8000-00805f9b34fb';
const testCharacteristic = '0000fff1-0000-1000-8000-00805f9b34fb';

/// Automatic C-ABI replies for widget tests, with no native library or radio.
class UiBridge extends FakeNativeBridge {
  bool holdSubscription = false;
  int capabilitiesBits =
      Capability.passiveScan |
      Capability.activeScan |
      Capability.scanResponse |
      Capability.gattClient |
      Capability.rssiDuringConnection |
      Capability.mtuNegotiation |
      Capability.mtuQuery |
      Capability.pairing |
      Capability.autoConnect |
      Capability.adapterPowerControl;
  final unsupportedMethods = <String>{};
  var systemConnectedResults = [
    {'id': testDeviceId.json, 'name': 'System Sensor'},
  ];

  @override
  int send(
    ffi.Pointer<RblueHandle> handle,
    int reqId,
    String method,
    Map<String, Object?> params,
    Uint8List payload,
  ) {
    final result = super.send(handle, reqId, method, params, payload);
    if (method == 'subscribe' && holdSubscription) return result;
    scheduleMicrotask(() {
      if (disposed) return;
      if (unsupportedMethods.contains(method)) {
        failRequest(
          reqId,
          jsonEncode({
            'Unsupported': {'platform': 'test', 'feature': method},
          }),
        );
        return;
      }
      Object? response;
      switch (method) {
        case 'initialize':
          emitEvent(
            jsonEncode({
              'type': 'adapter_event',
              'event': {'state_changed': 'powered_on'},
            }),
          );
        case 'backend_version':
          response = 'widget-test-bridge';
        case 'capabilities':
          response = capabilitiesBits;
        case 'max_write_len':
          response = 244;
        case 'mtu':
          response = 247;
        case 'request_mtu':
          response = params['mtu'] ?? 185;
        case 'rssi':
          response = -48;
        case 'connect':
          emitEvent(
            jsonEncode({
              'type': 'adapter_event',
              'event': {'device_connected': params['device']},
            }),
          );
        case 'disconnect':
          emitEvent(
            jsonEncode({
              'type': 'adapter_event',
              'event': {
                'device_disconnected': [params['device'], 'local'],
              },
            }),
          );
        case 'system_connected_peripherals':
          response = systemConnectedResults;
        case 'discover_services':
          response = {
            'services': [
              // Repeated UUIDs must retain distinct service and characteristic instances.
              for (var index = 0; index < 2; index++)
                {
                  'uuid': testService,
                  'primary': true,
                  'characteristics': [
                    {
                      'uuid': testCharacteristic,
                      'properties': 'READ | WRITE | NOTIFY',
                      'descriptors': <String>[],
                    },
                  ],
                },
            ],
          };
        case 'read':
          respondTo(reqId, utf8.encode('Hello'));
          return;
      }
      respondTo(
        reqId,
        response == null ? [] : utf8.encode(jsonEncode(response)),
      );
    });
    return result;
  }

  void discover([DeviceId? id]) => emitEvent(
    jsonEncode({
      'type': 'adapter_event',
      'event': {
        'device_discovered': {
          'id': (id ?? testDeviceId).json,
          'advertisement': {
            'local_name': 'Test Sensor',
            'services': [testService],
            'rssi': -48,
            'is_connectable': true,
          },
        },
      },
    }),
  );

  void notify([List<int> bytes = const [84, 61, 50, 52]]) => emitEvent(
    jsonEncode({
      'type': 'notification',
      'device': testDeviceId.json,
      'notification': {
        'id': const CharacteristicId(
          service: ServiceId(uuid: testService, instance: 0),
          uuid: testCharacteristic,
          instance: 0,
        ).toJson(),
        'data': bytes,
      },
    }),
  );

  void emitSupervisor(Object event) =>
      emitEvent(jsonEncode({'type': 'supervisor_event', 'event': event}));
}

({AppState state, UiBridge bridge}) createUiFixture() {
  final bridge = UiBridge();
  final client = RblueClient.forBridge(bridge, bridge.create()!);
  return (
    state: AppState(
      clientFactory: () async => client,
      permissionRequester: () async => true,
    ),
    bridge: bridge,
  );
}
