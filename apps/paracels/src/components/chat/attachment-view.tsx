import { Image } from 'expo-image';
import { SymbolView } from 'expo-symbols';
import { useState } from 'react';
import { ActivityIndicator, Pressable, View } from 'react-native';

import { formatByteSize, isImageMime } from '@/chat/client/attachment-format';
import { openLocalAttachment, openRemoteAttachment } from '@/chat/client/attachments';
import { strings } from '@/chat/client/strings';
import { ThemedText } from '@/components/themed-text';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

export type ShownAttachment = {
  fileName: string;
  mimeType: string;
  byteSize: number;
  localUri: string | null;
  remoteId: string | null;
};

/**
 * An image thumbnail or a file row. Tapping downloads a remote file, then opens Quick Look.
 */
export function AttachmentView({
  attachment,
  apiOrigin,
  roomKey,
  textColor,
  accessibilityLabel,
}: {
  attachment: ShownAttachment;
  apiOrigin: string;
  roomKey: string | null;
  textColor: string;
  accessibilityLabel?: string;
}) {
  const theme = useTheme();
  const [aspectRatio, setAspectRatio] = useState(4 / 3);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const label = `${attachment.fileName}, ${formatByteSize(attachment.byteSize)}`;
  const imageUri = attachment.localUri ?? (attachment.remoteId === null ? null : `${apiOrigin}/api/attachments/${attachment.remoteId}`);
  const showImage = isImageMime(attachment.mimeType) && imageUri !== null;

  async function open(): Promise<void> {
    if (progress !== null) {
      return;
    }
    setError(null);
    if (attachment.localUri !== null) {
      const failed = await openLocalAttachment(attachment.localUri, attachment.fileName);
      setError(failed === null ? null : strings.previewFailed);
      return;
    }
    if (attachment.remoteId === null || roomKey === null) {
      setError(strings.downloadFailed);
      return;
    }
    setProgress(0);
    const failed = await openRemoteAttachment(apiOrigin, roomKey, attachment.remoteId, attachment.fileName, setProgress);
    setProgress(null);
    setError(failed === 'download_failed' ? strings.downloadFailed : failed === 'preview_failed' ? strings.previewFailed : null);
  }

  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? `${strings.openAttachment}: ${label}`}
      accessibilityHint={strings.openAttachmentHint}
      onPress={() => {
        void open();
      }}
      style={{ gap: Spacing.one }}>
      {showImage ? (
        <Image
          source={
            attachment.localUri !== null
              ? { uri: attachment.localUri }
              : { uri: imageUri ?? '', headers: roomKey === null ? undefined : { Authorization: `Bearer ${roomKey}` } }
          }
          contentFit="cover"
          style={{ width: '100%', aspectRatio, borderRadius: 12, backgroundColor: theme.backgroundElement }}
          onLoad={(event) => {
            const width = event.source?.width ?? 0;
            const height = event.source?.height ?? 0;
            if (width > 0 && height > 0) {
              setAspectRatio(width / height);
            }
          }}
        />
      ) : (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: Spacing.two, minHeight: 44 }}>
          <SymbolView name="doc" size={22} tintColor={textColor} />
          <View style={{ flexShrink: 1 }}>
            <ThemedText style={{ color: textColor }}>{attachment.fileName}</ThemedText>
            <ThemedText type="small" style={{ color: textColor }}>
              {formatByteSize(attachment.byteSize)}
            </ThemedText>
          </View>
        </View>
      )}
      {progress !== null ? (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: Spacing.two, minHeight: 44 }}>
          <ActivityIndicator color={theme.tint} />
          <ThemedText type="small" style={{ color: textColor }}>
            {strings.downloading(Math.round(progress * 100))}
          </ThemedText>
        </View>
      ) : null}
      {error !== null ? (
        <ThemedText type="small" themeColor="danger">
          {error}
        </ThemedText>
      ) : null}
    </Pressable>
  );
}
