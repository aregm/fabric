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
