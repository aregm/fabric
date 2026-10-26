import { ActivityIndicator, Pressable, View } from 'react-native';

import { displaySenderName, formatMessageTime } from '@/chat/client/format';
import { strings, bubbleAccessibilityLabel } from '@/chat/client/strings';
import type { VisibleMessage } from '@/chat/client/state';
import type { DemoUserId } from '@/chat/shared/users';
import { ThemedText } from '@/components/themed-text';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

/**
 * One message, sent or still in the outbox.
 */
export function MessageBubble({
  item,
  currentUserId,
  onRetry,
  onDelete,
}: {
  item: VisibleMessage;
  currentUserId: DemoUserId;
  onRetry: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const theme = useTheme();
  const isOwn =
    item.kind === 'sent' ? item.message.senderId === currentUserId : item.item.senderId === currentUserId;
  const senderId = item.kind === 'sent' ? item.message.senderId : item.item.senderId;
  const text =
    item.kind === 'sent' && typeof item.message.payload.text === 'string'
      ? item.message.payload.text
      : item.kind === 'outbox'
        ? item.item.text
        : '';
  const time = formatMessageTime(item.kind === 'sent' ? item.message.serverReceivedAt : item.item.clientCreatedAt);
  const sender = isOwn ? strings.you : displaySenderName(senderId);
  const statusLabel =
    item.kind === 'outbox'
      ? item.item.state === 'waiting'
        ? strings.waiting
        : item.item.state === 'sending'
          ? strings.sending
          : strings.notSent
      : isOwn
        ? strings.sent
        : null;

  return (
    <View
      accessible
      accessibilityRole="text"
      accessibilityLabel={bubbleAccessibilityLabel(sender, time, statusLabel, text)}
      style={{
        alignSelf: isOwn ? 'flex-end' : 'flex-start',
        maxWidth: '82%',
        gap: Spacing.one,
      }}>
      <ThemedText type="small" themeColor="textSecondary">
        {sender} · {time}
      </ThemedText>
      <View
        style={{
          backgroundColor: isOwn ? theme.ownBubble : theme.otherBubble,
          paddingHorizontal: Spacing.three,
          paddingVertical: Spacing.two,
          borderRadius: 18,
          borderCurve: 'continuous',
        }}>
        <ThemedText selectable style={{ color: isOwn ? theme.ownText : theme.text }}>
          {text}
        </ThemedText>
      </View>
      {isOwn && item.kind === 'sent' ? (
        <ThemedText type="small" themeColor="textSecondary">
          {strings.sent}
        </ThemedText>
      ) : null}
      {item.kind === 'outbox' ? (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: Spacing.two, minHeight: 44 }}>
          {item.item.state === 'sending' ? <ActivityIndicator size="small" color={theme.tint} /> : null}
          <ThemedText type="small" themeColor={item.item.state === 'failed' ? 'danger' : 'textSecondary'}>
            {statusLabel}
          </ThemedText>
          {item.item.state === 'failed' ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={strings.retry}
              accessibilityHint={strings.retryHint}
              onPress={() => onRetry(item.item.id)}
              style={{ minHeight: 44, justifyContent: 'center', paddingHorizontal: Spacing.one }}>
              <ThemedText type="smallBold" themeColor="tint">
                {strings.retry}
              </ThemedText>
            </Pressable>
          ) : null}
          {item.item.state === 'failed' || item.item.state === 'waiting' ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={strings.delete}
              accessibilityHint={strings.deleteHint}
              onPress={() => onDelete(item.item.id)}
              style={{ minHeight: 44, justifyContent: 'center', paddingHorizontal: Spacing.one }}>
              <ThemedText type="smallBold" themeColor="danger">
                {strings.delete}
              </ThemedText>
            </Pressable>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}
