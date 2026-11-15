import { ActivityIndicator, Pressable, View } from 'react-native';

import { formatByteSize } from '@/chat/client/attachment-format';
import { displaySenderName, formatMessageTime } from '@/chat/client/format';
import { strings, bubbleAccessibilityLabel } from '@/chat/client/strings';
import type { VisibleMessage } from '@/chat/client/state';
import { validatePlainPayload } from '@/chat/shared/validate';
import type { DemoUserId } from '@/chat/shared/users';
import { AttachmentView, type ShownAttachment } from '@/components/chat/attachment-view';
import { ThemedText } from '@/components/themed-text';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

/**
 * One message, sent or still in the outbox.
 */
export function MessageBubble({
  item,
  currentUserId,
  apiOrigin,
  roomKey,
  onRetry,
  onDelete,
}: {
  item: VisibleMessage;
  currentUserId: DemoUserId;
  apiOrigin: string;
  roomKey: string | null;
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
  const attachment = shownAttachment(item);
  const attachmentLabel = attachment === null ? null : `${attachment.fileName}, ${formatByteSize(attachment.byteSize)}`;

  return (
    <View
      accessible={attachment === null}
      accessibilityRole="text"
      accessibilityLabel={bubbleAccessibilityLabel(sender, time, statusLabel, text, attachmentLabel)}
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
        {text.length > 0 ? (
          <ThemedText selectable style={{ color: isOwn ? theme.ownText : theme.text }}>
            {text}
          </ThemedText>
        ) : null}
        {attachment !== null ? (
          <AttachmentView
            attachment={attachment}
            apiOrigin={apiOrigin}
            roomKey={roomKey}
            textColor={isOwn ? theme.ownText : theme.text}
            accessibilityLabel={bubbleAccessibilityLabel(sender, time, statusLabel, text, attachmentLabel)}
          />
        ) : null}
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

function shownAttachment(item: VisibleMessage): ShownAttachment | null {
  if (item.kind === 'outbox') {
    const attachment = item.item.attachment;
    if (attachment === null) {
      return null;
    }
    return {
      fileName: attachment.fileName,
      mimeType: attachment.mimeType,
      byteSize: attachment.byteSize,
      localUri: attachment.uri,
      remoteId: null,
    };
  }
  const plain = validatePlainPayload(item.message.payload);
  if (!plain.ok || plain.value.attachment === null) {
    return null;
  }
  return {
    fileName: plain.value.attachment.fileName,
    mimeType: plain.value.attachment.mimeType,
    byteSize: plain.value.attachment.byteSize,
    localUri: null,
    remoteId: item.message.attachmentId,
  };
}
