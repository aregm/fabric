/**
 * Chat state: history pages, live messages, acks and the outbox, merged by
 * message ID (docs/paracels-chat.md, section 5.1).
 */

import { MAIN_ROOM_ID, PLAIN_PAYLOAD_FORMAT, type StoredMessage } from '../shared/protocol.ts';
import type { OutboxItem } from './outbox.ts';

export type HistoryStatus = 'loading' | 'ready' | 'error';

export type ChatState = {
  databaseId: string | null;
  latestSeq: number;
  oldestSeq: number | null;
  hasOlder: boolean;
  historyStatus: HistoryStatus;
  historyError: string | null;
  isLoadingOlder: boolean;
  messages: Record<string, StoredMessage>;
  outbox: OutboxItem[];
};

export type VisibleMessage =
  | { key: string; kind: 'sent'; message: StoredMessage }
  | { key: string; kind: 'outbox'; item: OutboxItem };

export type ChatAction =
  | { type: 'history_loading' }
  | { type: 'older_loading' }
  | { type: 'history_failed'; message: string }
  | {
      type: 'history_page';
      databaseId: string;
      messages: StoredMessage[];
      latestSeq: number;
      hasMore: boolean;
      mode: 'first' | 'older' | 'catchup';
    }
  | { type: 'live_message'; message: StoredMessage }
  | { type: 'acked'; id: string; seq: number; serverReceivedAt: string }
  | { type: 'outbox_changed'; items: OutboxItem[] }
  | { type: 'reset_history' };

/**
 * The empty chat state, before the first history page.
 */
export function createChatState(): ChatState {
  return {
    databaseId: null,
    latestSeq: 0,
    oldestSeq: null,
    hasOlder: false,
    historyStatus: 'loading',
    historyError: null,
    isLoadingOlder: false,
    messages: {},
    outbox: [],
  };
}

/**
 * Applies one event to the chat state.
 * @param state The current state.
 * @param action The event.
 */
export function reduceChat(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case 'history_loading':
      return { ...state, historyStatus: 'loading', historyError: null };
    case 'older_loading':
      return { ...state, isLoadingOlder: true };
    case 'history_failed':
      return {
        ...state,
        historyStatus: state.databaseId === null && Object.keys(state.messages).length === 0 ? 'error' : 'ready',
        historyError: action.message,
        isLoadingOlder: false,
      };
    case 'history_page':
      return applyHistoryPage(state, action);
    case 'live_message':
      return rememberMessage(state, action.message);
    case 'acked':
      return applyAck(state, action);
    case 'outbox_changed':
      return { ...state, outbox: action.items.filter((item) => state.messages[item.id] === undefined) };
    case 'reset_history':
      return {
        ...state,
        databaseId: null,
        latestSeq: 0,
        oldestSeq: null,
        hasOlder: false,
        historyStatus: 'loading',
        historyError: null,
        isLoadingOlder: false,
        messages: {},
      };
  }
}

/**
 * Sent messages in sequence order, then unsent ones in device time order.
 * @param state The current state.
 */
export function selectVisibleMessages(state: ChatState): VisibleMessage[] {
  const sent = Object.values(state.messages)
    .sort((left, right) => left.seq - right.seq)
    .map((message) => ({ key: message.id, kind: 'sent' as const, message }));
  const unsent = state.outbox
    .filter((item) => state.messages[item.id] === undefined)
    .sort((left, right) => left.clientCreatedAt.localeCompare(right.clientCreatedAt))
    .map((item) => ({ key: item.id, kind: 'outbox' as const, item }));
  return [...sent, ...unsent];
}

/**
 * The highest sequence number the app holds, or 0 when it holds none.
 * @param state The current state.
 */
export function highestHeldSeq(state: ChatState): number {
  return Object.values(state.messages).reduce((highest, message) => Math.max(highest, message.seq), 0);
}

function applyHistoryPage(
  state: ChatState,
  action: Extract<ChatAction, { type: 'history_page' }>,
): ChatState {
  if (state.databaseId !== null && action.databaseId !== state.databaseId) {
    return applyHistoryPage(
      {
        ...state,
        databaseId: null,
        latestSeq: 0,
        oldestSeq: null,
        hasOlder: false,
        messages: {},
      },
      action,
    );
  }
  let next: ChatState = { ...state, databaseId: action.databaseId, latestSeq: Math.max(state.latestSeq, action.latestSeq) };
  for (const message of action.messages) {
    next = rememberMessage(next, message);
  }
  const seqs = Object.values(next.messages).map((message) => message.seq);
  const oldestSeq = seqs.length === 0 ? null : Math.min(...seqs);
  const hasOlder =
    action.mode === 'first'
      ? action.hasMore
      : action.mode === 'older'
        ? action.hasMore
        : state.hasOlder;
  return {
    ...next,
    oldestSeq,
    hasOlder,
    historyStatus: 'ready',
    historyError: null,
    isLoadingOlder: false,
  };
}

function rememberMessage(state: ChatState, message: StoredMessage): ChatState {
  const messages = { ...state.messages, [message.id]: message };
  return {
    ...state,
    latestSeq: Math.max(state.latestSeq, message.seq),
    oldestSeq: state.oldestSeq === null ? message.seq : Math.min(state.oldestSeq, message.seq),
    messages,
    outbox: state.outbox.filter((item) => item.id !== message.id),
  };
}

function applyAck(state: ChatState, action: Extract<ChatAction, { type: 'acked' }>): ChatState {
  const existing = state.messages[action.id];
  if (existing !== undefined) {
    return { ...state, outbox: state.outbox.filter((item) => item.id !== action.id) };
  }
  const item = state.outbox.find((entry) => entry.id === action.id);
  if (item === undefined) {
    return state;
  }
  const stored: StoredMessage = {
    id: item.id,
    roomId: MAIN_ROOM_ID,
    senderId: item.senderId,
    clientCreatedAt: item.clientCreatedAt,
    attachmentId: null,
    payload: { format: PLAIN_PAYLOAD_FORMAT, text: item.text, attachment: null },
    seq: action.seq,
    serverReceivedAt: action.serverReceivedAt,
  };
  return rememberMessage(state, stored);
}
