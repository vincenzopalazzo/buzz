import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/unread_badge/should_notify_for_event.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  const self =
      '1111111111111111111111111111111111111111111111111111111111111111';
  const agent =
      '2222222222222222222222222222222222222222222222222222222222222222';

  NostrEvent message(String content, {int kind = EventKind.streamMessage}) =>
      NostrEvent(
        id: 'event-1',
        pubkey: agent,
        createdAt: 1,
        kind: kind,
        tags: const [
          ['h', 'channel-1'],
          ['p', self],
        ],
        content: content,
        sig: 'sig',
      );

  test('Sonar settlement rows never notify; receipts still do', () {
    expect(shouldNotifyForEvent(message('⚡PAYDONE|2|pay-1'), self), isFalse);
    expect(
      shouldNotifyForEvent(
        message('⚡PAYDONE|1|pay-1', kind: EventKind.streamMessageV2),
        self,
      ),
      isFalse,
    );
    expect(
      shouldNotifyForEvent(message('Paid.\n⚡PAY|1|pay-1|21'), self),
      isTrue,
    );
    expect(
      shouldNotifyForEvent(message('Done.\n⚡PAYDONE|2|pay-1'), self),
      isTrue,
    );
  });
}
