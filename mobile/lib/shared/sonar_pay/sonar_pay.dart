/// Sonar chat payment receipts, rendered by Buzz without a wallet.
///
/// Wire format (Sonar `docs/SONAR-PAYMENTS.md`, decoder `SonarPay.kt`): plain
/// control strings carried as the *entire* content of an ordinary message.
///
///     ⚡PAY|1|<id>|<sats>              receipt, shown as a gold bubble
///     ⚡PAYDONE|2|<id>                 settled, no preimage available
///     ⚡PAYDONE|2|<id>|<preimage_hex>  settled, with the Lightning preimage
///
/// Mirrors `desktop/src/features/messages/lib/sonarPay.ts`.
library;

import 'package:flutter/foundation.dart';

@immutable
sealed class SonarPayLine {
  const SonarPayLine(this.id);
  final String id;
}

final class SonarPayReceipt extends SonarPayLine {
  const SonarPayReceipt(super.id, this.sats);
  final int sats;
}

final class SonarPayDone extends SonarPayLine {
  const SonarPayDone(super.id, [this.preimage]);
  final String? preimage;
}

/// Render state of a `⚡PAY` row.
@immutable
class SonarPayView {
  const SonarPayView({
    required this.id,
    required this.sats,
    required this.settled,
    this.preimage,
  });

  final String id;
  final int sats;
  final bool settled;
  final String? preimage;

  @override
  bool operator ==(Object other) =>
      other is SonarPayView &&
      other.id == id &&
      other.sats == sats &&
      other.settled == settled &&
      other.preimage == preimage;

  @override
  int get hashCode => Object.hash(id, sats, settled, preimage);
}

final _hex64 = RegExp(r'^[0-9a-fA-F]{64}$');
final _digits = RegExp(r'^[0-9]+$');

/// Mirror of Sonar's `PayLine.decode`. Returns null for anything else.
SonarPayLine? decodeSonarPayLine(String content) {
  final parts = content.split('|');
  if (parts.length < 3) return null;
  final id = parts[2];
  if (id.isEmpty) return null;
  final version = parts[1];
  switch (parts[0]) {
    case '⚡PAY':
      if (version != '1' || parts.length < 4) return null;
      if (!_digits.hasMatch(parts[3])) return null;
      final sats = int.tryParse(parts[3]);
      if (sats == null || sats <= 0) return null;
      return SonarPayReceipt(id, sats);
    case '⚡PAYDONE':
      if (version == '1') {
        return parts.length == 3 ? SonarPayDone(id) : null;
      }
      if (version == '2') {
        if (parts.length == 3) return SonarPayDone(id);
        if (parts.length == 4 && _hex64.hasMatch(parts[3])) {
          return SonarPayDone(id, parts[3].toLowerCase());
        }
      }
      return null;
    default:
      return null;
  }
}

/// A message's payment lines split from its text. Sonar sends each line as
/// a whole message; an agent usually wraps them in a sentence, so each line
/// may also sit on its own line inside a longer message (no leading space;
/// trailing whitespace ignored).
@immutable
class SonarPayContent {
  const SonarPayContent(this.text, this.pays, this.dones);

  /// The message without its payment lines, trimmed.
  final String text;
  final List<SonarPayReceipt> pays;
  final List<SonarPayDone> dones;
}

SonarPayContent? parseSonarPayContent(String content) {
  final pays = <SonarPayReceipt>[];
  final dones = <SonarPayDone>[];
  final kept = <String>[];
  for (final rawLine in content.split('\n')) {
    final line = decodeSonarPayLine(rawLine.trimRight());
    switch (line) {
      case final SonarPayReceipt pay:
        pays.add(pay);
      case final SonarPayDone done:
        dones.add(done);
      case null:
        kept.add(rawLine);
    }
  }
  if (pays.isEmpty && dones.isEmpty) return null;
  return SonarPayContent(kept.join('\n').trim(), pays, dones);
}

/// A message of only `⚡PAYDONE` lines is a hidden control row that only
/// settles a `⚡PAY`.
bool isSonarPayControlLine(String content) {
  final parsed = parseSonarPayContent(content);
  return parsed != null &&
      parsed.pays.isEmpty &&
      parsed.dones.isNotEmpty &&
      parsed.text.isEmpty;
}

String _key(String pubkey, String id) => '${pubkey.toLowerCase()}:$id';

/// Settlements keyed by signer and id. A `⚡PAYDONE` only settles a `⚡PAY`
/// signed by the same key, so nobody can mark someone else's receipt paid.
/// A DONE with a preimage wins over one without, in any arrival order.
Map<String, String?> collectSonarPaySettlements(
  Iterable<({String pubkey, String content})> messages,
) {
  final settlements = <String, String?>{};
  for (final message in messages) {
    final parsed = parseSonarPayContent(message.content);
    if (parsed == null) continue;
    for (final done in parsed.dones) {
      final key = _key(message.pubkey, done.id);
      if (!settlements.containsKey(key) ||
          (settlements[key] == null && done.preimage != null)) {
        settlements[key] = done.preimage;
      }
    }
  }
  return settlements;
}

/// Text plus payment bubbles for a message that carries `⚡PAY` lines.
@immutable
class SonarPayMessage {
  const SonarPayMessage({required this.text, required this.receipts});

  /// Remaining message text, shown above the bubbles (may be empty).
  final String text;
  final List<SonarPayView> receipts;

  @override
  bool operator ==(Object other) =>
      other is SonarPayMessage &&
      other.text == text &&
      listEquals(other.receipts, receipts);

  @override
  int get hashCode => Object.hash(text, Object.hashAll(receipts));
}

/// The view for a message with payment lines, or null for any other
/// message (and for DONE-only control rows, which are hidden).
SonarPayMessage? resolveSonarPayMessage(
  String content,
  String signerPubkey,
  Map<String, String?> settlements,
) {
  final parsed = parseSonarPayContent(content);
  if (parsed == null || (parsed.pays.isEmpty && parsed.text.isEmpty)) {
    return null;
  }
  return SonarPayMessage(
    text: parsed.text,
    receipts: [
      for (final pay in parsed.pays)
        SonarPayView(
          id: pay.id,
          sats: pay.sats,
          settled: settlements.containsKey(_key(signerPubkey, pay.id)),
          preimage: settlements[_key(signerPubkey, pay.id)],
        ),
    ],
  );
}
