import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/models.dart';

void main() {
  test('characteristic instances count only matching UUIDs', () {
    const a = Characteristic(uuid: 'a', properties: 0, descriptors: []);
    const b = Characteristic(uuid: 'b', properties: 0, descriptors: []);
    const set = ServiceSet(
      services: [
        Service(uuid: 'service', primary: true, characteristics: [a, b, a]),
        Service(uuid: 'other', primary: true, characteristics: [b]),
        Service(uuid: 'service', primary: true, characteristics: [a]),
      ],
    );
    final ids = set.identifiedCharacteristics.map((entry) => entry.$1).toList();
    expect(ids.map((id) => id.instance), [0, 0, 1, 0, 0]);
    expect(ids.map((id) => id.service.instance), [0, 0, 0, 0, 1]);
    expect(set.find('b'), ids[1]);
  });

  test('advertisement retains service data and optional radio fields', () {
    final data = AdvertisementData.fromJson(const {
      'services': <String>[],
      'service_data': [
        [
          '180f',
          [1, 2],
        ],
      ],
      'tx_power': -4,
      'adv_flags': 6,
    });
    expect(data.serviceData.single.$1, '180f');
    expect(data.serviceData.single.$2, [1, 2]);
    expect(data.txPower, -4);
    expect(data.flags, 6);
  });

  group('DeviceId', () {
    test('parses all three serde variants', () {
      final mac = DeviceId.fromJson(const {
        'mac': [
          [2, 0, 0, 202, 254, 1],
          'random_static',
        ],
      });
      expect(mac.isMac, isTrue);
      expect(mac.macBytes, [2, 0, 0, 202, 254, 1]);
      expect(mac.addressType, AddressType.randomStatic);
      expect(mac.display, '02:00:00:CA:FE:01');

      final uuid = DeviceId.fromJson(const {
        'uuid': '2f6f2cda-2f0f-4f1f-8f3f-3f4f5f6f7f8f',
      });
      expect(uuid.display, '2f6f2cda-2f0f-4f1f-8f3f-3f4f5f6f7f8f');
      expect(
        DeviceId.fromJson(const {'web': 'device-42'}).display,
        'device-42',
      );
    });

    test('json round-trips through the exact wire form', () {
      final wire = jsonEncode(const {
        'mac': [
          [2, 0, 0, 0, 0, 1],
          'resolvable_private',
        ],
      });
      final parsed = DeviceId.fromJson(jsonDecode(wire)!);
      expect(jsonEncode(parsed.json), wire);
      expect(parsed.addressType, AddressType.resolvablePrivate);
    });
  });

  group('AdapterEvent', () {
    test('decodes every variant of the rblue-c envelope', () {
      final discovered =
          AdapterEvent.fromJson(const {
                'device_discovered': {
                  'id': {
                    'mac': [
                      [2, 0, 0, 0, 0, 1],
                      'random_static',
                    ],
                  },
                  'advertisement': {
                    'local_name': 'sensor',
                    'services': ['00000000-0000-0000-0000-00000000180f'],
                    'service_data': [],
                    'manufacturer_data': null,
                    'tx_power': null,
                    'adv_flags': null,
                    'rssi': -40,
                    'is_connectable': true,
                  },
                },
              })
              as AdapterDeviceDiscovered;
      expect(discovered.device.advertisement.localName, 'sensor');
      expect(
        discovered.device.advertisement.services.single,
        '00000000-0000-0000-0000-00000000180f',
      );

      const updated = {
        'device_updated': [
          {
            'mac': [
              [2, 0, 0, 0, 0, 1],
              'random_static',
            ],
          },
          -55,
        ],
      };
      final update = AdapterEvent.fromJson(updated) as AdapterDeviceUpdated;
      expect(update.rssi, -55);

      const disconnected = {
        'device_disconnected': [
          {
            'mac': [
              [2, 0, 0, 0, 0, 1],
              'random_static',
            ],
          },
          {'platform': 133},
        ],
      };
      final drop =
          AdapterEvent.fromJson(disconnected) as AdapterDeviceDisconnected;
      expect(drop.reason!.platformCode, 133);
      expect(drop.reason!.display, 'platform 133');

      final connected =
          AdapterEvent.fromJson(const {
                'device_connected': {
                  'uuid': '2f6f2cda-2f0f-4f1f-8f3f-3f4f5f6f7f8f',
                },
              })
              as AdapterDeviceConnected;
      expect(connected.id.display, '2f6f2cda-2f0f-4f1f-8f3f-3f4f5f6f7f8f');

      final stateChanged =
          AdapterEvent.fromJson(const {'state_changed': 'powered_on'})
              as AdapterStateChanged;
      expect(stateChanged.state, AdapterState.poweredOn);
    });
  });

  group('ServiceSet', () {
    test('parses the exact rblue-c payload (bitflags string form)', () {
      const payload =
          '{"services":[{"uuid":"00000000-0000-0000-0000-00000000180f",'
          '"primary":true,"characteristics":[{"uuid":"00000000-0000-0000-0000-000000002a19",'
          '"properties":"READ | WRITE_WITHOUT_RESPONSE | WRITE | NOTIFY | INDICATE",'
          '"descriptors":[]}]}]}';
      final set = ServiceSet.fromJson(
        (jsonDecode(payload)! as Map).cast<String, Object?>(),
      );
      final characteristic = set.allCharacteristics.single;
      expect(characteristic.readable, isTrue);
      expect(characteristic.writable, isTrue);
      expect(characteristic.writableWithoutResponse, isTrue);
      expect(characteristic.notifiable, isTrue);
      expect(set.find('00000000-0000-0000-0000-000000002a19'), isNotNull);
    });
  });

  group('RblueException', () {
    test('decodes Rust unit variants from JSON strings', () {
      for (final kind in [
        RblueErrorKind.notConnected,
        RblueErrorKind.timeout,
        RblueErrorKind.disconnected,
      ]) {
        final raw = '"${kind.wire}"';
        final error = RblueException.fromJson(raw);
        expect(error.kind, kind);
        expect(error.message, kind.wire);
        expect(error.rawJson, raw);
      }
    });

    test('parses Platform, InvalidLength, and InvalidArgument', () {
      final platform = RblueException.fromJson(
        '{"Platform":{"code":133,"message":"GATT_ERROR"}}',
      );
      expect(platform.kind, RblueErrorKind.platform);
      expect(platform.platformCode, 133);
      expect(platform.isAndroidGattStatus133, isTrue);

      final length = RblueException.fromJson(
        '{"InvalidLength":{"len":600,"max":514}}',
      );
      expect(length.message, 'data length 600 exceeds max 514');

      final argument = RblueException.fromJson(
        '{"InvalidArgument":{"message":"bad"}}',
      );
      expect(argument.message, 'bad');

      expect(
        RblueException.fromJson(
          '{"NotFound":{"mac":[[2,0,0,0,0,1],"random_static"]}}',
        ).kind,
        RblueErrorKind.notFound,
      );
      expect(
        RblueException.fromJson('{"NotConnected":null}').kind,
        RblueErrorKind.notConnected,
      );
    });
  });

  group('DeviceNotification', () {
    test('decodes the notification envelope fields', () {
      const envelope = {
        'type': 'notification',
        'device': {
          'mac': [
            [2, 0, 0, 0, 0, 1],
            'random_static',
          ],
        },
        'notification': {
          'id': {
            'service': {
              'uuid': '00000000-0000-0000-0000-00000000180f',
              'instance': 0,
            },
            'uuid': '00000000-0000-0000-0000-000000002a19',
            'instance': 0,
          },
          'data': [104, 105],
        },
      };
      final notification = DeviceNotification.fromJson(envelope);
      expect(notification.dataAsText, 'hi');
      expect(notification.dataAsHex, '68 69');
      expect(notification.id.uuid, '00000000-0000-0000-0000-000000002a19');
    });
  });

  group('Capability', () {
    test('keeps published bit positions and reserves bit 13', () {
      expect(Capability.passiveScan, 1 << 0);
      expect(Capability.backgroundScan, 1 << 12);
      expect(Capability.adapterPowerControl, 1 << 14);
      expect(
        Capability.has(
          Capability.adapterPowerControl,
          Capability.adapterPowerControl,
        ),
        isTrue,
      );
      expect(
        Capability.has(Capability.gattClient, Capability.adapterPowerControl),
        isFalse,
      );
    });
  });
}
