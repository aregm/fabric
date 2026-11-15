/**
 * The one WebSocket connection (docs/paracels-chat.md, D24 and section 7.2).
 * The app is lenient: unknown server frames are ignored.
 */

import { PROTOCOL_VERSION, type ClientFrame, type ServerFrame } from '../shared/protocol.ts';
import { parseServerFrame } from '../shared/validate.ts';
import { reconnectDelayMs, type ChatClientConfig } from './config.ts';

export type ConnectionStatus = 'idle' | 'connecting' | 'connected' | 'disconnected';

export type AppLifecycle = {
  /**
   * Calls `onActive` when the app returns to the foreground.
   * @returns A function that removes the listener.
   */
  subscribe(onActive: () => void): () => void;
};

export type ChatSocket = {
  status(): ConnectionStatus;
  connect(): void;
  disconnect(): void;
  send(frame: Exclude<ClientFrame, { type: 'hello' }>): boolean;
  subscribe(listener: (event: ChatSocketEvent) => void): () => void;
};

export type ChatSocketEvent =
  | { type: 'status'; status: ConnectionStatus }
  | { type: 'frame'; frame: ServerFrame };

export type ChatSocketOptions = {
  config: ChatClientConfig;
  WebSocket?: typeof WebSocket;
  lifecycle?: AppLifecycle;
  random?: () => number;
  schedule?: (work: () => void, delayMs: number) => () => void;
};

/**
 * Creates the socket client. It does not connect until {@link ChatSocket.connect} is called.
 * @param options See {@link ChatSocketOptions}.
 */
export function createChatSocket(options: ChatSocketOptions): ChatSocket {
  const Socket = options.WebSocket ?? WebSocket;
  const random = options.random ?? Math.random;
  const schedule = options.schedule ?? defaultSchedule;
  const listeners = new Set<(event: ChatSocketEvent) => void>();
  let status: ConnectionStatus = 'idle';
  let socket: WebSocket | null = null;
  let shouldRun = false;
  let attempt = 0;
  let cancelReconnect: (() => void) | null = null;
  let cancelPing: (() => void) | null = null;
  let cancelPong: (() => void) | null = null;
  const unsubscribeLifecycle = options.lifecycle?.subscribe(() => {
    if (shouldRun && status !== 'connected' && status !== 'connecting') {
      connectNow();
    }
  });

  function emit(event: ChatSocketEvent): void {
    for (const listener of listeners) {
      listener(event);
    }
  }

  function setStatus(next: ConnectionStatus): void {
    if (status === next) {
      return;
    }
    status = next;
    emit({ type: 'status', status });
  }

  function clearTimers(): void {
    cancelReconnect?.();
    cancelReconnect = null;
    cancelPing?.();
    cancelPing = null;
    cancelPong?.();
    cancelPong = null;
  }

  function dropSocket(): void {
    clearTimers();
    if (socket === null) {
      return;
    }
    const current = socket;
    socket = null;
    current.onopen = null;
    current.onmessage = null;
    current.onerror = null;
    current.onclose = null;
    if (current.readyState === WebSocket.CONNECTING || current.readyState === WebSocket.OPEN) {
      current.close();
    }
  }

  function scheduleReconnect(): void {
    if (!shouldRun || cancelReconnect !== null) {
      return;
    }
    const delay = reconnectDelayMs(attempt, random());
    attempt += 1;
    cancelReconnect = schedule(() => {
      cancelReconnect = null;
      connectNow();
    }, delay);
  }

  function startPing(): void {
    cancelPing?.();
    cancelPing = schedule(() => {
      if (socket === null || socket.readyState !== WebSocket.OPEN) {
        return;
      }
      socket.send(JSON.stringify({ type: 'ping' } satisfies ClientFrame));
      cancelPong?.();
      cancelPong = schedule(() => {
        if (shouldRun) {
          dropSocket();
          setStatus('disconnected');
          scheduleReconnect();
        }
      }, options.config.pongTimeoutMs);
      startPing();
    }, options.config.pingIntervalMs);
  }

  function connectNow(): void {
    const roomKey = options.config.roomKey;
    if (!shouldRun || roomKey === null) {
      return;
    }
    cancelReconnect?.();
    cancelReconnect = null;
    dropSocket();
    setStatus('connecting');
    let next: WebSocket;
    try {
      next = new Socket(options.config.socketUrl);
    } catch {
      setStatus('disconnected');
      scheduleReconnect();
      return;
    }
    socket = next;
    next.onopen = () => {
      if (socket !== next) {
        return;
      }
      next.send(
        JSON.stringify({
          type: 'hello',
          protocol: PROTOCOL_VERSION,
          roomKey,
        } satisfies ClientFrame),
      );
    };
    next.onmessage = (event: MessageEvent) => {
      if (socket !== next || typeof event.data !== 'string') {
        return;
      }
      const parsed = parseServerFrame(event.data);
      if (parsed.kind !== 'frame') {
        return;
      }
      if (parsed.frame.type === 'welcome') {
        attempt = 0;
        setStatus('connected');
        startPing();
      }
      if (parsed.frame.type === 'pong') {
        cancelPong?.();
        cancelPong = null;
      }
      emit({ type: 'frame', frame: parsed.frame });
    };
    next.onerror = () => undefined;
    next.onclose = () => {
      if (socket !== next) {
        return;
      }
      socket = null;
      clearTimers();
      if (!shouldRun) {
        setStatus('idle');
        return;
      }
      setStatus('disconnected');
      scheduleReconnect();
    };
  }

  return {
    status: () => status,
    connect() {
      shouldRun = true;
      if (status === 'connected' || status === 'connecting') {
        return;
      }
      connectNow();
    },
    disconnect() {
      shouldRun = false;
      dropSocket();
      setStatus('idle');
      unsubscribeLifecycle?.();
    },
    send(frame) {
      if (socket === null || socket.readyState !== WebSocket.OPEN || status !== 'connected') {
        return false;
      }
      socket.send(JSON.stringify(frame));
      return true;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

function defaultSchedule(work: () => void, delayMs: number): () => void {
  const handle = setTimeout(work, delayMs);
  return () => clearTimeout(handle);
}
