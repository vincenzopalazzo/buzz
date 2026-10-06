import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/sonar_pay/sonar_pay.dart';
import 'package:buzz/shared/sonar_pay/sonar_pay_bubble.dart';

const _preimage =
    '0000000000000000000000000000000000000000000000000000000000000000';

NostrEvent _msg(String id, String content, {String pubkey = 'alice'}) =>
    NostrEvent(
      id: id,
      pubkey: pubkey,
      createdAt: 1000,
      kind: EventKind.streamMessage,
      tags: const [
        ['h', 'ch1'],
      ],
      content: content,
      sig: '',
    );

void main() {
  group('decodeSonarPayLine mirrors Sonar', () {
    test('decodes PAY and both DONE versions', () {
      final pay = decodeSonarPayLine('⚡PAY|1|abc-123|21');
      expect(pay, isA<SonarPayReceipt>());
      expect((pay! as SonarPayReceipt).sats, 21);
      expect(decodeSonarPayLine('⚡PAYDONE|2|abc-123'), isA<SonarPayDone>());
      final done = decodeSonarPayLine('⚡PAYDONE|2|abc-123|$_preimage');
      expect((done! as SonarPayDone).preimage, _preimage);
      expect(decodeSonarPayLine('⚡PAYDONE|1|abc-123'), isA<SonarPayDone>());
    });

    test('everything else stays plain text', () {
      for (final content in [
        '⚡PAY|2|abc|21',
        '⚡PAY|1|abc|0',
        '⚡PAY|1|abc|-5',
        '⚡PAY|1|abc',
        '⚡PAYDONE|3|abc',
        '⚡PAYDONE|1|abc|extra',
        '⚡PAYDONE|2|abc|nothex',
        ' ⚡PAYDONE|1|abc-123',
        '⚡PAYCLAIM|1|abc|21',
        'hello',
      ]) {
        expect(decodeSonarPayLine(content), isNull, reason: content);
      }
    });
  });

  group('formatTimeline', () {
    test('DONE settles PAY in any order and is hidden', () {
      final messages = formatTimeline([
        _msg('d1', '⚡PAYDONE|2|p1|$_preimage'),
        _msg('m1', '⚡PAY|1|p1|21'),
      ]);
      expect(messages.map((m) => m.id), ['m1']);
      expect(
        messages.single.sonarPay,
        const SonarPayView(
          id: 'p1',
          sats: 21,
          settled: true,
          preimage: _preimage,
        ),
      );
    });

    test('a DONE from another signer does not settle the PAY', () {
      final messages = formatTimeline([
        _msg('m1', '⚡PAY|1|p1|21'),
        _msg('d1', '⚡PAYDONE|2|p1', pubkey: 'mallory'),
      ]);
      expect(messages.single.sonarPay?.settled, isFalse);
    });

    test('plain messages carry no payment state', () {
      final messages = formatTimeline([_msg('m1', 'hello')]);
      expect(messages.single.sonarPay, isNull);
    });
  });

  testWidgets('bubble shows amount, status and a copyable proof', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(
      const ProviderScope(
        child: MaterialApp(
          home: Scaffold(
            body: SonarPayBubble(
              pay: SonarPayView(
                id: 'p1',
                sats: 2100,
                settled: true,
                preimage: _preimage,
              ),
              mine: false,
            ),
          ),
        ),
      ),
    );
    expect(find.text('2,100'), findsOneWidget);
    expect(find.text('Received'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('sonar-pay-copy-preimage')),
      findsOneWidget,
    );
    expect(
      find.bySemanticsLabel('Lightning payment: Paid 2,100 sats'),
      findsOneWidget,
    );
    expect(find.bySemanticsLabel('Copy payment preimage'), findsOneWidget);
    // The receipt itself is not a button; only the proof control is.
    final receipt = tester.getSemantics(
      find.bySemanticsLabel('Lightning payment: Paid 2,100 sats'),
    );
    expect(receipt.flagsCollection.isButton, isFalse);
    semantics.dispose();
  });
}
