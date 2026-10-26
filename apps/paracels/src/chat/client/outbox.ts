/**
 * The outbox: unsent messages, behind an interface (docs/paracels-chat.md,
 * sections 8 and 9). The app uses the expo-sqlite store. This memory store is
 * what the worker tests use.
 */

import { RECONNECT_MAX_DELAY_MS, RECONNECT_MIN_DELAY_MS } from '../shared/limits.ts';
import type { DemoUserId } from '../shared/users.ts';

export type OutboxState = 'waiting' | 'sending' | 'failed';

/** A file kept with an unsent message. `sha256` is set once the upload succeeds. */
export type OutboxAttachment = {
  id: string;
  /** Copy in the app's documents directory. */
  uri: string;
  fileName: string;
  mimeType: string;
  byteSize: number;
  sha256: string | null;
};

export type OutboxItem = {
  id: string;
  senderId: DemoUserId;
  clientCreatedAt: string;
  text: string;
  attachment: OutboxAttachment | null;
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
 * An in-memory outbox for tests. The app opens the expo-sqlite store instead.
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

/** The outcome of uploading an attachment before its message is sent. */
export type PrepareResult =
  | { ok: true; item: OutboxItem }
  | { ok: false; retryable: boolean; code: string };

export type OutboxWorkerOptions = {
  store: OutboxStore;
  isConnected: () => boolean;
  send: (item: OutboxItem) => void;
  onChange: (items: OutboxItem[]) => void;
  /** Called after a row is deleted, so the app can delete its file copy. */
  onRemoved?: (item: OutboxItem) => void;
  /**
   * Uploads an attachment that has no SHA-256 yet. The item is already `sending`.
   * Absent for text-only messages.
   */
  prepare?: (item: OutboxItem) => Promise<PrepareResult>;
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
  let flight = 0;
  let cancelTimeout: (() => void) | null = null;
  const retriedUpload = new Set<string>();

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

  function stillCurrent(generation: number, id: string): boolean {
    return generation === flight && inFlightId === id;
  }

  function beginSend(generation: number, sending: OutboxItem): void {
    if (!stillCurrent(generation, sending.id)) {
      return;
    }
    options.send(sending);
    const startedAt = now();
    cancelTimeout = schedule(() => {
      if (!stillCurrent(generation, sending.id)) {
        return;
      }
      if (now() - startedAt < options.ackTimeoutMs) {
        return;
      }
      inFlightId = null;
      clearTimeoutHandle();
      mark(sending.id, 'failed', 'ack_timeout');
      kick();
    }, options.ackTimeoutMs);
  }

  async function finishUpload(generation: number, sending: OutboxItem): Promise<void> {
    const prepare = options.prepare;
    if (prepare === undefined) {
      return;
    }
    let prepared: PrepareResult;
    try {
      prepared = await prepare(sending);
    } catch {
      prepared = { ok: false, retryable: true, code: 'upload_failed' };
    }
    if (!stillCurrent(generation, sending.id)) {
      return;
    }
    if (!prepared.ok) {
      inFlightId = null;
      clearTimeoutHandle();
      if (prepared.retryable) {
        mark(sending.id, 'waiting');
        if (options.isConnected()) {
          const attempts = options.store.get(sending.id)?.attempts ?? 1;
          cancelTimeout = schedule(kick, retryDelayMs(attempts));
        }
        return;
      }
      mark(sending.id, 'failed', prepared.code);
      kick();
      return;
    }
    const uploaded = prepared.item.attachment;
    if (uploaded === null || uploaded.sha256 === null) {
      inFlightId = null;
      mark(sending.id, 'failed', 'upload_failed');
      kick();
      return;
    }
    const next = { ...prepared.item, state: 'sending' as const, failureCode: null };
    options.store.put(next);
    publish();
    beginSend(generation, next);
  }

  function kick(): void {
    if (inFlightId !== null || !options.isConnected()) {
      return;
    }
    const next = options.store.list().find((item) => item.state === 'waiting');
    if (next === undefined) {
      return;
    }
    const generation = ++flight;
    inFlightId = next.id;
    mark(next.id, 'sending');
    const sending = options.store.get(next.id);
    if (sending === undefined) {
      inFlightId = null;
      return;
    }
    if (sending.attachment !== null && sending.attachment.sha256 === null && options.prepare !== undefined) {
      void finishUpload(generation, sending);
      return;
    }
    beginSend(generation, sending);
  }

  return {
    kick,
    handleAck(id) {
      if (inFlightId === id) {
        inFlightId = null;
        clearTimeoutHandle();
      }
      retriedUpload.delete(id);
      const item = options.store.get(id);
      options.store.remove(id);
      if (item !== undefined) {
        options.onRemoved?.(item);
      }
      publish();
      kick();
    },
    handleReject(id, code) {
      if (inFlightId === id) {
        inFlightId = null;
        clearTimeoutHandle();
      }
      const item = options.store.get(id);
      if (
        code === 'attachment_missing' &&
        item?.attachment !== null &&
        item?.attachment !== undefined &&
        item.attachment.sha256 !== null &&
        !retriedUpload.has(id)
      ) {
        retriedUpload.add(id);
        options.store.put({
          ...item,
          state: 'waiting',
          failureCode: null,
          attachment: { ...item.attachment, sha256: null },
        });
        publish();
        kick();
        return;
      }
      mark(id, 'failed', code);
      kick();
    },
    handleDisconnect() {
      flight += 1;
      if (inFlightId !== null) {
        const id = inFlightId;
        inFlightId = null;
        clearTimeoutHandle();
        mark(id, 'waiting');
      }
    },
    dispose() {
      flight += 1;
      inFlightId = null;
      clearTimeoutHandle();
    },
  };
}

function retryDelayMs(attempts: number): number {
  return Math.min(RECONNECT_MAX_DELAY_MS, RECONNECT_MIN_DELAY_MS * 2 ** Math.max(0, attempts - 1));
}

function defaultSchedule(work: () => void, delayMs: number): () => void {
  const handle = setTimeout(work, delayMs);
  return () => clearTimeout(handle);
}
