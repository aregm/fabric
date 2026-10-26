/**
 * React context for the chat screen and its header (D23).
 */

import { createContext, use, type ReactNode } from 'react';

import { useChat, type ChatController } from './use-chat.ts';

const ChatContext = createContext<ChatController | null>(null);

/**
 * Runs the chat for the whole Stack.
 * @param children The screens that read {@link useChatContext}.
 */
export function ChatProvider({ children }: { children: ReactNode }) {
  const chat = useChat();
  return <ChatContext value={chat}>{children}</ChatContext>;
}

/**
 * The running chat. Must be used under {@link ChatProvider}.
 */
export function useChatContext(): ChatController {
  const chat = use(ChatContext);
  if (chat === null) {
    throw new Error('useChatContext must be used under ChatProvider.');
  }
  return chat;
}
