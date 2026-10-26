/**
 * Wires the HTTP client, the socket, the outbox and the reducer for the screen
 * (docs/paracels-chat.md, sections 7.5, 8 and 9).
 */

import { randomUUID } from 'expo-crypto';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { AppState } from 'react-native';

import { HISTORY_CATCH_UP_PAGE_SIZE, HISTORY_DEFAULT_PAGE_SIZE } from '../shared/limits.ts';
import { MAIN_ROOM_ID, PLAIN_PAYLOAD_FORMAT, type StoredMessage } from '../shared/protocol.ts';
import { isDemoUserId, type DemoUserId } from '../shared/users.ts';
import { createChatApi, type ChatApi } from './api.ts';
import { readBundleLocation, readChatConfig, type ChatClientConfig } from './config.ts';
import { createMemoryOutbox, createOutboxWorker, type OutboxItem, type OutboxStore, type OutboxWorker } from './outbox.ts';
import { createChatSocket, type AppLifecycle, type ChatSocket, type ConnectionStatus } from './socket.ts';
import {
  createChatState,
  highestHeldSeq,
  reduceChat,
  selectVisibleMessages,
  type ChatAction,
  type ChatState,
  type VisibleMessage,
} from './state.ts';
import { readStoredUserId, writeStoredUserId } from './user-store.ts';

export type ChatController = {
  config: ChatClientConfig;
  userId: DemoUserId;
  connection: ConnectionStatus;
  state: ChatState;
  messages: VisibleMessage[];
  switchUser: (userId: DemoUserId) => void;
  sendText: (text: string) => void;
  retry: (id: string) => void;
  remove: (id: string) => void;
  loadOlder: () => void;
  reloadHistory: () => void;
};

function appLifecycle(): AppLifecycle {
  return {
    subscribe(onActive) {
      const subscription = AppState.addEventListener('change', (status) => {
        if (status === 'active') {
          onActive();
        }
      });
      return () => subscription.remove();
    },
  };
}

function envelopeFromOutbox(item: OutboxItem) {
  return {
    id: item.id,
    roomId: MAIN_ROOM_ID,
    senderId: item.senderId,
    clientCreatedAt: item.clientCreatedAt,
    attachmentId: null,
    payload: { format: PLAIN_PAYLOAD_FORMAT, text: item.text, attachment: null },
  };
}

async function fetchHistoryPages(
  api: ChatApi,
  mode: 'first' | 'older' | 'catchup',
  state: ChatState,
): Promise<Extract<ChatAction, { type: 'history_page' }> | { type: 'history_failed'; message: string }> {
  const cursor =
    mode === 'older' && state.oldestSeq !== null
      ? { before: state.oldestSeq }
      : mode === 'catchup'
        ? { after: highestHeldSeq(state) }
        : null;
  const limit = mode === 'catchup' ? HISTORY_CATCH_UP_PAGE_SIZE : HISTORY_DEFAULT_PAGE_SIZE;
  const first = await api.readHistory({ cursor, limit });
  if (!first.ok) {
    return { type: 'history_failed', message: first.error.message };
  }
  const collected: StoredMessage[] = [...first.value.messages];
  let latestSeq = first.value.latestSeq;
  let hasMore = first.value.hasMore;
  if (mode === 'catchup') {
    while (hasMore) {
      const after = collected.reduce((highest, message) => Math.max(highest, message.seq), highestHeldSeq(state));
      const next = await api.readHistory({ cursor: { after }, limit });
      if (!next.ok || next.value.messages.length === 0) {
        break;
      }
      collected.push(...next.value.messages);
      latestSeq = next.value.latestSeq;
      hasMore = next.value.hasMore;
    }
  }
  return {
    type: 'history_page',
    databaseId: first.value.databaseId,
    messages: collected,
    latestSeq,
    hasMore: mode === 'catchup' ? state.hasOlder : hasMore,
    mode,
  };
}

/**
 * The hook that runs the chat for one screen.
 */
export function useChat(): ChatController {
  const config = useMemo(() => readChatConfig(process.env, readBundleLocation()), []);
  const [userId, setUserId] = useState<DemoUserId>(readStoredUserId);
  const [connection, setConnection] = useState<ConnectionStatus>(config.roomKey === null ? 'disconnected' : 'idle');
  const [state, dispatch] = useReducer(reduceChat, undefined, createChatState);
  const api = useMemo<ChatApi | null>(
    () => (config.roomKey === null ? null : createChatApi({ roomKey: config.roomKey, apiOrigin: config.apiOrigin })),
    [config],
  );
  const storeRef = useRef<OutboxStore>(createMemoryOutbox());
  const workerRef = useRef<OutboxWorker | null>(null);
  const socketRef = useRef<ChatSocket | null>(null);
  const stateRef = useRef(state);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  const publishOutbox = useCallback(() => {
    dispatch({ type: 'outbox_changed', items: storeRef.current.list() });
  }, []);

  const load = useCallback(
    async (mode: 'first' | 'older' | 'catchup') => {
      if (api === null) {
        dispatch({ type: 'history_failed', message: 'The room key is not set.' });
        return;
      }
      const action = await fetchHistoryPages(api, mode, stateRef.current);
      dispatch(action);
    },
    [api],
  );

  useEffect(() => {
    const store = storeRef.current;
    const socket = createChatSocket({ config, lifecycle: appLifecycle() });
    socketRef.current = socket;
    const worker = createOutboxWorker({
      store,
      isConnected: () => socket.status() === 'connected',
      send(item) {
        socket.send({ type: 'send', message: envelopeFromOutbox(item) });
      },
      onChange: (items) => dispatch({ type: 'outbox_changed', items }),
      ackTimeoutMs: config.ackTimeoutMs,
    });
    workerRef.current = worker;

    const unsubscribe = socket.subscribe((event) => {
      if (event.type === 'status') {
        setConnection(event.status);
        if (event.status === 'disconnected' || event.status === 'idle') {
          worker.handleDisconnect();
        }
        return;
      }
      const frame = event.frame;
      switch (frame.type) {
        case 'welcome':
          if (stateRef.current.databaseId !== null && frame.databaseId !== stateRef.current.databaseId) {
            dispatch({ type: 'reset_history' });
          }
          void load('catchup');
          worker.kick();
          return;
        case 'ack':
          dispatch({ type: 'acked', id: frame.id, seq: frame.seq, serverReceivedAt: frame.serverReceivedAt });
          worker.handleAck(frame.id);
          return;
        case 'reject':
          worker.handleReject(frame.id, frame.code);
          return;
        case 'message':
          dispatch({ type: 'live_message', message: frame.message });
          return;
        default:
          return;
      }
    });

    if (config.roomKey !== null) {
      socket.connect();
    }
    void load('first');

    return () => {
      unsubscribe();
      worker.dispose();
      socket.disconnect();
      socketRef.current = null;
      workerRef.current = null;
    };
  }, [config, load]);

  const switchUser = useCallback((next: DemoUserId) => {
    if (!isDemoUserId(next)) {
      return;
    }
    writeStoredUserId(next);
    setUserId(next);
  }, []);

  const sendText = useCallback(
    (text: string) => {
      const trimmed = text.trim();
      if (trimmed.length === 0 || trimmed.length > config.maxTextLength) {
        return;
      }
      const item: OutboxItem = {
        id: randomUUID(),
        senderId: userId,
        clientCreatedAt: new Date().toISOString(),
        text: trimmed,
        state: 'waiting',
        failureCode: null,
        attempts: 0,
      };
      storeRef.current.put(item);
      publishOutbox();
      workerRef.current?.kick();
    },
    [config.maxTextLength, publishOutbox, userId],
  );

  const retry = useCallback(
    (id: string) => {
      const item = storeRef.current.get(id);
      if (item === undefined || item.state !== 'failed') {
        return;
      }
      storeRef.current.put({ ...item, state: 'waiting', failureCode: null });
      publishOutbox();
      workerRef.current?.kick();
    },
    [publishOutbox],
  );

  const remove = useCallback(
    (id: string) => {
      storeRef.current.remove(id);
      publishOutbox();
    },
    [publishOutbox],
  );

  const loadOlder = useCallback(() => {
    const current = stateRef.current;
    if (!current.hasOlder || current.isLoadingOlder || current.oldestSeq === null) {
      return;
    }
    dispatch({ type: 'older_loading' });
    void load('older');
  }, [load]);

  const reloadHistory = useCallback(() => {
    dispatch({ type: 'history_loading' });
    void load('first');
  }, [load]);

  return {
    config,
    userId,
    connection,
    state,
    messages: selectVisibleMessages(state),
    switchUser,
    sendText,
    retry,
    remove,
    loadOlder,
    reloadHistory,
  };
}
