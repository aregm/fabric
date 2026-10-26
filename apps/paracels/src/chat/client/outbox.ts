/**
 * The outbox: unsent messages, behind an interface (docs/paracels-chat.md,
 * sections 8 and 9). Phase 3 keeps them in memory; Phase 4 writes the same
 * shape to expo-sqlite.
 */

import type { DemoUserId } from '../shared/users.ts';

export type OutboxState = 'waiting' | 'sending' | 'failed';

export type OutboxItem = {
  id: string;
  senderId: DemoUserId;
  clientCreatedAt: string;
  text: string;
  state: OutboxState;
  failureCode: string | null;
  attempts: number;
};

export type OutboxStore = {
  list(): OutboxItem[];
  get(id: string): OutboxItem | undefined;
  put(item: OutboxItem): void;
  remove(id: string): void;
};

/**
 * An in-memory outbox. Waiting messages are lost if the app is closed (Phase 4).
 */
export function createMemoryOutbox(): OutboxStore {
  const items = new Map<string, OutboxItem>();
  return {
    list() {
      return [...items.values()].sort((left, right) => left.clientCreatedAt.localeCompare(right.clientCreatedAt));
    },
    get(id) {
      return items.get(id);
    },
    put(item) {
      items.set(item.id, item);
    },
    remove(id) {
      items.delete(id);
    },
  };
}

export type OutboxWorker = {
  /** Sends the oldest waiting message if the socket is connected and nothing is in flight. */
  kick(): void;
  handleAck(id: string): void;
  handleReject(id: string, code: string): void;
  handleDisconnect(): void;
  dispose(): void;
};

export type OutboxWorkerOptions = {
  store: OutboxStore;
  isConnected: () => boolean;
  send: (item: OutboxItem) => void;
  onChange: (items: OutboxItem[]) => void;
  ackTimeoutMs: number;
  now?: () => number;
  schedule?: (work: () => void, delayMs: number) => () => void;
};

/**
 * Sends waiting messages one at a time, oldest first, while the socket is up.
 * @param options See {@link OutboxWorkerOptions}.
 */
export function createOutboxWorker(options: OutboxWorkerOptions): OutboxWorker {
  const now = options.now ?? Date.now;
  const schedule = options.schedule ?? defaultSchedule;
  let inFlightId: string | null = null;
  let cancelTimeout: (() => void) | null = null;

  function publish(): void {
    options.onChange(options.store.list());
  }

  function clearTimeoutHandle(): void {
    cancelTimeout?.();
    cancelTimeout = null;
  }

  function mark(id: string, state: OutboxState, failureCode: string | null = null): void {
    const item = options.store.get(id);
    if (item === undefined) {
      return;
    }
    options.store.put({
      ...item,
      state,
      failureCode,
      attempts: state === 'sending' ? item.attempts + 1 : item.attempts,
    });
    publish();
  }

  function kick(): void {
    if (inFlightId !== null || !options.isConnected()) {
      return;
    }
    const next = options.store.list().find((item) => item.state === 'waiting');
    if (next === undefined) {
      return;
    }
    inFlightId = next.id;
    mark(next.id, 'sending');
    const sending = options.store.get(next.id);
    if (sending === undefined) {
      inFlightId = null;
      return;
    }
    options.send(sending);
    const startedAt = now();
    cancelTimeout = schedule(() => {
      if (inFlightId !== next.id) {
        return;
      }
      if (now() - startedAt < options.ackTimeoutMs) {
        return;
      }
      inFlightId = null;
      clearTimeoutHandle();
      mark(next.id, 'failed', 'ack_timeout');
      kick();
    }, options.ackTimeoutMs);
  }

  return {
    kick,
    handleAck(id) {
      if (inFlightId === id) {
        inFlightId = null;
        clearTimeoutHandle();
      }
      options.store.remove(id);
      publish();
      kick();
    },
    handleReject(id, code) {
      if (inFlightId === id) {
        inFlightId = null;
        clearTimeoutHandle();
      }
      mark(id, 'failed', code);
      kick();
    },
    handleDisconnect() {
      if (inFlightId !== null) {
        const id = inFlightId;
        inFlightId = null;
        clearTimeoutHandle();
        mark(id, 'waiting');
      }
    },
    dispose() {
      inFlightId = null;
      clearTimeoutHandle();
    },
  };
}

function defaultSchedule(work: () => void, delayMs: number): () => void {
  const handle = setTimeout(work, delayMs);
  return () => clearTimeout(handle);
}
