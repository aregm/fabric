import { useRef } from 'react';
import { ActivityIndicator, FlatList, Pressable, View, type NativeScrollEvent, type NativeSyntheticEvent } from 'react-native';

import type { VisibleMessage } from '@/chat/client/state';
import { strings } from '@/chat/client/strings';
import type { DemoUserId } from '@/chat/shared/users';
import { MessageBubble } from '@/components/chat/message-bubble';
import { ThemedText } from '@/components/themed-text';
import { Spacing } from '@/constants/theme';
import { useTheme } from '@/hooks/use-theme';

/** How close to the latest message still counts as following the conversation. */
const STICK_TO_BOTTOM_PX = 80;

/**
 * Message list anchored to the newest row. Older pages load when you scroll to the top.
 * The list is not inverted: iOS 26 blurs an inverted scroll view across the whole screen.
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
  apiOrigin,
  roomKey,
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
  apiOrigin: string;
  roomKey: string | null;
}) {
  const theme = useTheme();
  const listRef = useRef<FlatList<VisibleMessage>>(null);
  const stickToBottom = useRef(true);

  const scrollToLatest = () => {
    if (stickToBottom.current) {
      listRef.current?.scrollToEnd({ animated: false });
    }
  };

  const onScroll = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    const distanceFromBottom = contentSize.height - layoutMeasurement.height - contentOffset.y;
    stickToBottom.current = distanceFromBottom < STICK_TO_BOTTOM_PX;
  };

  return (
    <FlatList
      ref={listRef}
      style={{ flex: 1 }}
      data={messages}
      keyExtractor={(item) => item.key}
      keyboardShouldPersistTaps="handled"
      contentInsetAdjustmentBehavior="automatic"
      maintainVisibleContentPosition={{ minIndexForVisible: 0 }}
      contentContainerStyle={{
        paddingHorizontal: Spacing.three,
        paddingVertical: Spacing.two,
        gap: Spacing.three,
        flexGrow: 1,
        justifyContent: 'flex-end',
      }}
      onScroll={onScroll}
      scrollEventThrottle={16}
      onContentSizeChange={scrollToLatest}
      onLayout={scrollToLatest}
      onStartReached={hasOlder ? onLoadOlder : undefined}
      onStartReachedThreshold={0.2}
      renderItem={({ item }) => (
        <MessageBubble
          item={item}
          currentUserId={currentUserId}
          apiOrigin={apiOrigin}
          roomKey={roomKey}
          onRetry={onRetry}
          onDelete={onDelete}
        />
      )}
      ListHeaderComponent={
        isLoadingOlder ? (
          <View style={{ paddingVertical: Spacing.two }}>
            <ActivityIndicator color={theme.tint} />
          </View>
        ) : undefined
      }
      ListFooterComponent={<View style={{ height: Spacing.three }} />}
      ListEmptyComponent={
        <View
          style={{
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
