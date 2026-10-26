import { useState } from 'react';
import { Pressable, TextInput, View } from 'react-native';

import { strings } from '@/chat/client/strings';
import { ThemedText } from '@/components/themed-text';
import { ThemedView } from '@/components/themed-view';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

const COUNTER_THRESHOLD = 200;

/**
 * Multiline composer. Attachments join it in Phase 5.
 */
export function Composer({
  maxLength,
  onSend,
}: {
  maxLength: number;
  onSend: (text: string) => void;
}) {
  const theme = useTheme();
  const [text, setText] = useState('');
  const remaining = maxLength - text.length;
  const canSend = text.trim().length > 0;

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
      {remaining <= COUNTER_THRESHOLD ? (
        <ThemedText type="small" themeColor="textSecondary" style={{ fontVariant: ['tabular-nums'] }}>
          {strings.characterCount(text.length, maxLength)}
        </ThemedText>
      ) : null}
      <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: Spacing.two }}>
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
            onSend(text);
            setText('');
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
