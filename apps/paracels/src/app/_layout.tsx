import { DarkTheme, DefaultTheme, ThemeProvider, Stack } from 'expo-router';
import * as SplashScreen from 'expo-splash-screen';
import { useEffect } from 'react';
import { useColorScheme } from 'react-native';

import { ChatProvider } from '@/chat/client/chat-context';
import { strings } from '@/chat/client/strings';

SplashScreen.preventAutoHideAsync();

export default function RootLayout() {
  const colorScheme = useColorScheme();

  useEffect(() => {
    void SplashScreen.hideAsync();
  }, []);

  return (
    <ThemeProvider value={colorScheme === 'dark' ? DarkTheme : DefaultTheme}>
      <ChatProvider>
        <Stack>
          <Stack.Screen name="index" options={{ title: strings.chatTitle }} />
        </Stack>
      </ChatProvider>
    </ThemeProvider>
  );
}
