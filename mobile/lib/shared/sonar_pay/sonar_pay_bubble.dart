import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';
import 'sonar_pay.dart';

/// Sonar's gold payment receipt bubble (`PayBubble` in SonarPayViews.kt).
///
/// Shows what the payer reported. Buzz holds no wallet and never pays; the
/// preimage, when present, can be copied as proof for the payee to check in
/// their own wallet.
class SonarPayBubble extends ConsumerWidget {
  const SonarPayBubble({super.key, required this.pay, required this.mine});

  final SonarPayView pay;

  /// True when the current user signed the `⚡PAY` line.
  final bool mine;

  static const _gold = Color(0xFFF5B83D);
  static const _goldSoft = Color(0xFFFDE7B0);
  static const _onGold = Color(0xFF3D2A00);

  /// Author-relative status: in a Buzz channel the viewer is rarely the
  /// payee, so this says what the author did (the row header names them).
  String get _status => pay.settled ? 'Paid' : 'Payment pending';

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final amount = _formatSats(pay.sats);
    final preimage = pay.preimage;
    return Semantics(
      container: true,
      label:
          'Lightning payment: ${pay.settled ? 'Paid' : 'Sending'} $amount sats',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          ExcludeSemantics(
            child: Container(
              key: const ValueKey('sonar-pay-bubble'),
              constraints: const BoxConstraints(minWidth: 190),
              padding: const EdgeInsets.fromLTRB(
                Grid.twelve,
                Grid.twelve,
                Grid.xs,
                Grid.twelve,
              ),
              decoration: BoxDecoration(
                color: _gold,
                borderRadius: BorderRadius.only(
                  topLeft: const Radius.circular(Radii.container),
                  topRight: const Radius.circular(Radii.container),
                  bottomLeft: Radius.circular(
                    mine ? Radii.container : Radii.sm,
                  ),
                  bottomRight: Radius.circular(
                    mine ? Radii.sm : Radii.container,
                  ),
                ),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    width: Grid.lg,
                    height: Grid.lg,
                    alignment: Alignment.center,
                    decoration: const BoxDecoration(
                      color: _goldSoft,
                      shape: BoxShape.circle,
                    ),
                    child: Text(
                      '₿',
                      style: context.textTheme.titleMedium?.copyWith(
                        color: _onGold,
                        fontWeight: FontWeight.w800,
                      ),
                    ),
                  ),
                  const SizedBox(width: Grid.twelve),
                  Text(
                    amount,
                    style: context.textTheme.titleLarge?.copyWith(
                      color: _onGold,
                      fontWeight: FontWeight.w800,
                    ),
                  ),
                  const SizedBox(width: Grid.quarter),
                  Text(
                    'sats',
                    style: context.textTheme.labelMedium?.copyWith(
                      color: _onGold.withValues(alpha: 0.7),
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: Grid.quarter),
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                LucideIcons.zap,
                size: 12,
                color: context.colors.onSurfaceVariant,
              ),
              const SizedBox(width: Grid.quarter),
              ExcludeSemantics(
                child: Text(
                  _status,
                  style: context.textTheme.labelSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ),
              if (pay.settled && preimage != null) ...[
                ExcludeSemantics(
                  child: Text(
                    ' · ',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
                // Own node: the copy action must not turn the whole receipt
                // into one button with a merged label.
                Semantics(
                  container: true,
                  button: true,
                  label: 'Copy payment preimage',
                  excludeSemantics: true,
                  child: InkWell(
                    key: const ValueKey('sonar-pay-copy-preimage'),
                    onTap: () =>
                        Clipboard.setData(ClipboardData(text: preimage)),
                    child: Text(
                      'proof',
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        decoration: TextDecoration.underline,
                      ),
                    ),
                  ),
                ),
              ],
            ],
          ),
        ],
      ),
    );
  }
}

String _formatSats(int sats) {
  final digits = sats.toString();
  final buffer = StringBuffer();
  for (var i = 0; i < digits.length; i++) {
    if (i > 0 && (digits.length - i) % 3 == 0) buffer.write(',');
    buffer.write(digits[i]);
  }
  return buffer.toString();
}
