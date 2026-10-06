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

/// `⚡PAYDONE` lines are hidden control rows that only settle a `⚡PAY`.
bool isSonarPayControlLine(String content) =>
    decodeSonarPayLine(content) is SonarPayDone;

String _key(String pubkey, String id) => '${pubkey.toLowerCase()}:$id';

/// Settlements keyed by signer and id. A `⚡PAYDONE` only settles a `⚡PAY`
/// signed by the same key, so nobody can mark someone else's receipt paid.
/// A DONE with a preimage wins over one without, in any arrival order.
Map<String, String?> collectSonarPaySettlements(
  Iterable<({String pubkey, String content})> messages,
) {
  final settlements = <String, String?>{};
  for (final message in messages) {
    final line = decodeSonarPayLine(message.content);
    if (line is! SonarPayDone) continue;
    final key = _key(message.pubkey, line.id);
    if (!settlements.containsKey(key) ||
        (settlements[key] == null && line.preimage != null)) {
      settlements[key] = line.preimage;
    }
  }
  return settlements;
}

/// Bubble state for a message whose content is a `⚡PAY` line.
SonarPayView? resolveSonarPayView(
  String content,
  String signerPubkey,
  Map<String, String?> settlements,
) {
  final line = decodeSonarPayLine(content);
  if (line is! SonarPayReceipt) return null;
  final key = _key(signerPubkey, line.id);
  return SonarPayView(
    id: line.id,
    sats: line.sats,
    settled: settlements.containsKey(key),
    preimage: settlements[key],
  );
}
