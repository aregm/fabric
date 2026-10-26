import { Stack } from 'expo-router';
import { KeyboardAvoidingView, Platform, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { useChatContext } from '@/chat/client/chat-context';
import { strings } from '@/chat/client/strings';
import { Composer } from '@/components/chat/composer';
import { ConnectionBanner } from '@/components/chat/connection-banner';
import { MessageList } from '@/components/chat/message-list';
import { UserMenu, userMenuAccessibilityLabel } from '@/components/chat/user-menu';
import { ThemedView } from '@/components/themed-view';

export default function ChatScreen() {
  const chat = useChatContext();
  const insets = useSafeAreaInsets();

  return (
    <ThemedView style={{ flex: 1 }}>
      <Stack.Screen
        options={{
          title: strings.chatTitle,
          headerRight: () => (
            <View accessible accessibilityRole="button" accessibilityLabel={userMenuAccessibilityLabel(chat.userId)} accessibilityHint={strings.switchUserHint}>
              <UserMenu userId={chat.userId} onSwitch={chat.switchUser} />
            </View>
          ),
        }}
      />
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ConnectionBanner
          status={chat.connection}
          address={chat.config.socketDisplay}
          hasRoomKey={chat.config.roomKey !== null}
        />
        <MessageList
          messages={chat.messages}
          currentUserId={chat.userId}
          historyStatus={chat.state.historyStatus}
          historyError={chat.state.historyError}
          isLoadingOlder={chat.state.isLoadingOlder}
          hasOlder={chat.state.hasOlder}
          onRetryHistory={chat.reloadHistory}
          onLoadOlder={chat.loadOlder}
          onRetry={chat.retry}
          onDelete={chat.remove}
        />
        <View style={{ paddingBottom: insets.bottom }}>
          <Composer maxLength={chat.config.maxTextLength} onSend={chat.sendText} />
        </View>
      </KeyboardAvoidingView>
    </ThemedView>
  );
}
