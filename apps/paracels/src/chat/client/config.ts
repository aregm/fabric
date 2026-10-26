/**
 * Server addresses, the room key and the app's timeouts (docs/paracels-chat.md,
 * D13, D24, D26 and D27).
 */

import {
  ACK_TIMEOUT_MS,
  APP_PING_INTERVAL_MS,
  APP_PONG_TIMEOUT_MS,
  CHAT_LIMITS,
  HISTORY_CATCH_UP_PAGE_SIZE,
  HISTORY_DEFAULT_PAGE_SIZE,
  RECONNECT_JITTER,
  RECONNECT_MAX_DELAY_MS,
  RECONNECT_MIN_DELAY_MS,
} from '../shared/limits.ts';
import { DEFAULT_SOCKET_PORT, SOCKET_PATH } from '../shared/protocol.ts';

export type ChatLocation = {
  hostname: string;
  origin: string;
  protocol: string;
};

export type ChatClientConfig = {
  /** Origin of the API routes; empty means relative URLs against the bundle host. */
  apiOrigin: string;
  /** WebSocket URL, including the `/chat` path. */
  socketUrl: string;
  /** Host and port shown in the connection banner. */
  socketDisplay: string;
  roomKey: string | null;
  maxTextLength: number;
  historyPageSize: number;
  catchUpPageSize: number;
  ackTimeoutMs: number;
  pingIntervalMs: number;
  pongTimeoutMs: number;
};

export type Environment = Readonly<Record<string, string | undefined>>;

/**
 * Reads the chat settings for this device.
 * @param environment Usually `process.env`.
 * @param location The bundle origin, typically `window.location`.
 */
export function readChatConfig(environment: Environment, location: ChatLocation | null): ChatClientConfig {
  const roomKey = environment.EXPO_PUBLIC_CHAT_ROOM_KEY?.trim() || null;
  const override = environment.EXPO_PUBLIC_CHAT_SOCKET_URL?.trim() || null;
  const socketUrl = override ?? socketUrlFromLocation(location);
  return {
    apiOrigin: location?.origin ?? '',
    socketUrl,
    socketDisplay: displayAddress(socketUrl),
    roomKey,
    maxTextLength: CHAT_LIMITS.maxTextLength,
    historyPageSize: HISTORY_DEFAULT_PAGE_SIZE,
    catchUpPageSize: HISTORY_CATCH_UP_PAGE_SIZE,
    ackTimeoutMs: ACK_TIMEOUT_MS,
    pingIntervalMs: APP_PING_INTERVAL_MS,
    pongTimeoutMs: APP_PONG_TIMEOUT_MS,
  };
}

/**
 * Reads `window.location` when the Metro location polyfill is present.
 */
export function readBundleLocation(): ChatLocation | null {
  const location = globalThis.location;
  if (location === undefined || typeof location.hostname !== 'string' || location.hostname.length === 0) {
    return null;
  }
  return { hostname: location.hostname, origin: location.origin, protocol: location.protocol };
}

/**
 * The next reconnect delay, with ±20% jitter (D24).
 * @param attempt Zero-based failed-attempt count.
 * @param random A number in `[0, 1)`.
 */
export function reconnectDelayMs(attempt: number, random: number): number {
  const base = Math.min(RECONNECT_MAX_DELAY_MS, RECONNECT_MIN_DELAY_MS * 2 ** Math.max(0, attempt));
  const jitter = (random * 2 - 1) * RECONNECT_JITTER;
  return Math.max(0, Math.round(base * (1 + jitter)));
}

function socketUrlFromLocation(location: ChatLocation | null): string {
  const hostname = location?.hostname || 'localhost';
  const protocol = location?.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${hostname}:${DEFAULT_SOCKET_PORT}${SOCKET_PATH}`;
}

function displayAddress(socketUrl: string): string {
  try {
    const url = new URL(socketUrl);
    return `${url.hostname}:${url.port || String(DEFAULT_SOCKET_PORT)}`;
  } catch {
    return socketUrl;
  }
}
