/**
 * Limits shared by the app and the chat servers (docs/paracels-chat.md, section 6.4).
 */

/** The limits the socket server announces in its `welcome` frame. */
export type ChatLimits = {
  /**
   * Longest message text, counted in UTF-16 code units, the way JavaScript's
   * `String.length` and React Native's `maxLength` count characters.
   */
  maxTextLength: number;
  /** Largest attachment in bytes; 10 MB means 10,000,000 bytes, as iOS shows sizes. */
  maxAttachmentBytes: number;
  /** Largest WebSocket frame the socket server accepts, in bytes. */
  maxFrameBytes: number;
};

export const CHAT_LIMITS: Readonly<ChatLimits> = Object.freeze({
  maxTextLength: 4000,
  maxAttachmentBytes: 10 * 1000 * 1000,
  maxFrameBytes: 64 * 1024,
});

/** Messages per history page when the request names no `limit` (D14). */
export const HISTORY_DEFAULT_PAGE_SIZE = 50;

/** Most messages one history page may hold (D14). */
export const HISTORY_MAX_PAGE_SIZE = 200;

/** Time a new WebSocket connection has to send `hello`. */
export const HELLO_TIMEOUT_MS = 5 * 1000;

/** Interval between the socket server's WebSocket pings. */
export const SERVER_PING_INTERVAL_MS = 30 * 1000;

/** How long the app waits for an `ack` before marking the message failed (D13). */
export const ACK_TIMEOUT_MS = 10 * 1000;

/** How often the app pings the socket server (D24). */
export const APP_PING_INTERVAL_MS = 20 * 1000;

/** How long the app waits for a `pong` before reconnecting (D24). */
export const APP_PONG_TIMEOUT_MS = 10 * 1000;

/** First reconnect delay; it doubles up to `RECONNECT_MAX_DELAY_MS` (D24). */
export const RECONNECT_MIN_DELAY_MS = 1000;

/** Longest reconnect delay (D24). */
export const RECONNECT_MAX_DELAY_MS = 30 * 1000;

/** Jitter applied to the reconnect delay, as a fraction of that delay (D24). */
export const RECONNECT_JITTER = 0.2;

/** Messages per catch-up page after `welcome` (D14). */
export const HISTORY_CATCH_UP_PAGE_SIZE = 200;
