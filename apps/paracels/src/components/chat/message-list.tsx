import { ActivityIndicator, FlatList, Pressable, View } from 'react-native';

import type { VisibleMessage } from '@/chat/client/state';
import { strings } from '@/chat/client/strings';
import type { DemoUserId } from '@/chat/shared/users';
import { MessageBubble } from '@/components/chat/message-bubble';
import { ThemedText } from '@/components/themed-text';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

/**
 * Inverted message list: newest at the bottom, older pages when you scroll up.
 */
export function MessageList({
  messages,
  currentUserId,
  historyStatus,
  historyError,
  isLoadingOlder,
  hasOlder,
  onRetryHistory,
  onLoadOlder,
  onRetry,
  onDelete,
}: {
  messages: VisibleMessage[];
  currentUserId: DemoUserId;
  historyStatus: 'loading' | 'ready' | 'error';
  historyError: string | null;
  isLoadingOlder: boolean;
  hasOlder: boolean;
  onRetryHistory: () => void;
  onLoadOlder: () => void;
  onRetry: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const theme = useTheme();
  const data = [...messages].reverse();

  return (
    <FlatList
      inverted
      data={data}
      keyExtractor={(item) => item.key}
      keyboardShouldPersistTaps="handled"
      contentInsetAdjustmentBehavior="automatic"
      contentContainerStyle={{
        paddingHorizontal: Spacing.three,
        paddingVertical: Spacing.two,
        gap: Spacing.three,
        flexGrow: 1,
      }}
      onEndReached={hasOlder ? onLoadOlder : undefined}
      onEndReachedThreshold={0.2}
      renderItem={({ item }) => (
        <MessageBubble item={item} currentUserId={currentUserId} onRetry={onRetry} onDelete={onDelete} />
      )}
      ListFooterComponent={
        isLoadingOlder ? (
          <View style={{ paddingVertical: Spacing.two }}>
            <ActivityIndicator color={theme.tint} />
          </View>
        ) : undefined
      }
      ListEmptyComponent={
        <View
          style={{
            transform: [{ scaleY: -1 }],
            flexGrow: 1,
            justifyContent: 'center',
            alignItems: 'center',
            padding: Spacing.four,
            gap: Spacing.two,
          }}>
          {historyStatus === 'loading' ? <ActivityIndicator color={theme.tint} /> : null}
          {historyStatus === 'error' ? (
            <>
              <ThemedText selectable>{historyError ?? strings.historyFailed}</ThemedText>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={strings.retryHistory}
                onPress={onRetryHistory}
                style={{ minHeight: 44, justifyContent: 'center' }}>
                <ThemedText type="smallBold" themeColor="tint">
                  {strings.retryHistory}
                </ThemedText>
              </Pressable>
            </>
          ) : null}
          {historyStatus === 'ready' ? <ThemedText themeColor="textSecondary">{strings.emptyRoom}</ThemedText> : null}
        </View>
      }
    />
  );
}
