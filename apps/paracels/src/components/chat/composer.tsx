import { Button, Host, Menu } from '@expo/ui/swift-ui';
import { buttonStyle, labelStyle } from '@expo/ui/swift-ui/modifiers';
import { Image } from 'expo-image';
import { useState } from 'react';
import { Pressable, TextInput, View } from 'react-native';

import { formatByteSize, isImageMime } from '@/chat/client/attachment-format';
import { pickFile, pickFromLibrary, takePhoto, type PickedAttachment, type PickFailureCode } from '@/chat/client/attachments';
import { strings } from '@/chat/client/strings';
import { ThemedText } from '@/components/themed-text';
import { ThemedView } from '@/components/themed-view';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

const COUNTER_THRESHOLD = 200;

/**
 * Multiline composer with one optional attachment (section 12).
 */
export function Composer({
  maxLength,
  onSend,
}: {
  maxLength: number;
  onSend: (draft: { text: string; attachment: PickedAttachment | null }) => string | null;
}) {
  const theme = useTheme();
  const [text, setText] = useState('');
  const [attachment, setAttachment] = useState<PickedAttachment | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const remaining = maxLength - text.length;
  const canSend = text.trim().length > 0 || attachment !== null;

  async function pick(kind: 'library' | 'camera' | 'file'): Promise<void> {
    const result = kind === 'library' ? await pickFromLibrary() : kind === 'camera' ? await takePhoto() : await pickFile();
    if (!result.ok) {
      if (result.code === 'canceled') {
        setNotice(null);
      } else if (result.code === 'denied') {
        setNotice(kind === 'camera' ? strings.cameraDenied : strings.photoLibraryDenied);
      } else {
        setNotice(pickFailureMessage(result.code));
      }
      return;
    }
    setAttachment(result.attachment);
    setNotice(null);
  }

  return (
    <ThemedView
      type="background"
      style={{
        borderTopWidth: 1,
        borderTopColor: theme.backgroundSelected,
        paddingHorizontal: Spacing.three,
        paddingVertical: Spacing.two,
        gap: Spacing.one,
      }}>
      {notice !== null ? (
        <ThemedText type="small" themeColor="danger">
          {notice}
        </ThemedText>
      ) : null}
      {remaining <= COUNTER_THRESHOLD ? (
        <ThemedText type="small" themeColor="textSecondary" style={{ fontVariant: ['tabular-nums'] }}>
          {strings.characterCount(text.length, maxLength)}
        </ThemedText>
      ) : null}
      {attachment !== null ? (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: Spacing.two }}>
          {isImageMime(attachment.mimeType) ? (
            <Image source={{ uri: attachment.uri }} style={{ width: 56, height: 56, borderRadius: 8 }} contentFit="cover" />
          ) : (
            <ThemedText type="small" style={{ flex: 1 }}>
              {attachment.fileName} · {formatByteSize(attachment.byteSize)}
            </ThemedText>
          )}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={strings.removeAttachment}
            accessibilityHint={strings.removeAttachmentHint}
            onPress={() => setAttachment(null)}
            style={{ minWidth: 44, minHeight: 44, justifyContent: 'center' }}>
            <ThemedText type="smallBold" themeColor="danger">
              {strings.removeAttachment}
            </ThemedText>
          </Pressable>
        </View>
      ) : null}
      <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: Spacing.two }}>
        <View accessible accessibilityRole="button" accessibilityLabel={strings.attach} accessibilityHint={strings.attachHint}>
          <Host matchContents style={{ minWidth: 44, minHeight: 44, justifyContent: 'center' }}>
            <Menu label={strings.attach} systemImage="paperclip" modifiers={[buttonStyle('plain'), labelStyle('iconOnly')]}>
              <Button label={strings.photoLibrary} onPress={() => void pick('library')} />
              <Button label={strings.takePhoto} onPress={() => void pick('camera')} />
              <Button label={strings.chooseFile} onPress={() => void pick('file')} />
            </Menu>
          </Host>
        </View>
        <TextInput
          value={text}
          onChangeText={setText}
          maxLength={maxLength}
          multiline
          placeholder={strings.composerPlaceholder}
          placeholderTextColor={theme.textSecondary}
          accessibilityLabel={strings.composerPlaceholder}
          style={{
            flex: 1,
            minHeight: 44,
            maxHeight: 120,
            paddingHorizontal: Spacing.two,
            paddingVertical: Spacing.two,
            borderRadius: 12,
            borderCurve: 'continuous',
            backgroundColor: theme.backgroundElement,
            color: theme.text,
            fontSize: 16,
          }}
        />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={strings.send}
          accessibilityHint={strings.sendHint}
          disabled={!canSend}
          onPress={() => {
            const error = onSend({ text, attachment });
            if (error !== null) {
              setNotice(error);
              return;
            }
            setText('');
            setAttachment(null);
            setNotice(null);
          }}
          style={{
            minWidth: 44,
            minHeight: 44,
            justifyContent: 'center',
            alignItems: 'center',
            opacity: canSend ? 1 : 0.4,
          }}>
          <ThemedText type="smallBold" themeColor="tint">
            {strings.send}
          </ThemedText>
        </Pressable>
      </View>
    </ThemedView>
  );
}

function pickFailureMessage(code: PickFailureCode): string {
  switch (code) {
    case 'too_large':
      return strings.attachmentTooLarge;
    case 'denied':
      return strings.photoLibraryDenied;
    case 'unavailable':
      return strings.cameraUnavailable;
    case 'unreadable':
      return strings.attachmentUnreadable;
    case 'canceled':
      return '';
  }
}
