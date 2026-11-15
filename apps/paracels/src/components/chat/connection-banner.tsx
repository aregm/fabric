import { ThemedText } from '@/components/themed-text';
import { ThemedView } from '@/components/themed-view';
import { Spacing } from '@/constants/theme';
import type { ConnectionStatus } from '@/chat/client/socket';
import { strings } from '@/chat/client/strings';

/**
 * Shown under the header while the socket is not connected.
 */
export function ConnectionBanner({
  status,
  address,
  hasRoomKey,
}: {
  status: ConnectionStatus;
  address: string;
  hasRoomKey: boolean;
}) {
  if (hasRoomKey && status === 'connected') {
    return null;
  }
  const message = !hasRoomKey
    ? strings.missingRoomKey
    : status === 'connecting' || status === 'idle'
      ? strings.connecting
      : strings.unreachable(address);
  return (
    <ThemedView
      type="backgroundElement"
      accessibilityRole="text"
      accessibilityLiveRegion="polite"
      style={{ paddingHorizontal: Spacing.three, paddingVertical: Spacing.two }}>
      <ThemedText type="small" selectable>
        {message}
      </ThemedText>
    </ThemedView>
  );
}
