/**
 * The chat socket server (docs/paracels-chat.md, sections 5.3 and 7).
 * It logs messages and relays them. The API routes serve history and attachments.
 *
 * Run it with `npm run chat:socket`. Tests import `startChatSocketServer`.
 */

import type { IncomingMessage } from 'node:http';

import { WebSocketServer, type RawData, WebSocket } from 'ws';

import { CHAT_LIMITS, HELLO_TIMEOUT_MS, SERVER_PING_INTERVAL_MS } from '../src/chat/shared/limits.ts';
import {
  CLOSE_CODES,
  MAIN_ROOM_ID,
  DEFAULT_SOCKET_PORT,
  PROTOCOL_VERSION,
  SOCKET_PATH,
  type ConnectionErrorCode,
  type ErrorFrame,
  type MessageEnvelope,
  type ServerFrame,
} from '../src/chat/shared/protocol.ts';
import { parseClientFrame } from '../src/chat/shared/validate.ts';
import { createAttachmentReader } from '../src/chat/server/attachment-store.ts';
import { readRoomKey, resolveDatabasePath, resolveSocketPort } from '../src/chat/server/config.ts';
import { openChatDatabase, type ChatDatabase } from '../src/chat/server/database.ts';
import { createJsonLogger, type ChatLogger } from '../src/chat/server/log.ts';
import { createMessageReader, createMessageWriter } from '../src/chat/server/message-store.ts';
import { reviewMessage } from '../src/chat/server/policy.ts';
import { createRoomKeyCheck, type RoomKeyCheck } from '../src/chat/server/room-key.ts';

/** A running socket server. */
export type ChatSocketServer = {
  /** The port it is listening on. */
  port: number;
  /** Closes every connection with 1001, stops listening, and closes the database. */
  close: () => Promise<void>;
};

export type ChatSocketOptions = {
  database: ChatDatabase;
  /** Accepts the room key from `hello`. */
  isRoomKey: RoomKeyCheck;
  log: ChatLogger;
  /** Defaults to 8787. Use 0 in tests to let the operating system choose. */
  port?: number;
  /** Defaults to every interface. */
  host?: string;
  now?: () => Date;
  helloTimeoutMs?: number;
  /** How often to ping, and how long a connection may stay silent before it is dropped. */
  pingIntervalMs?: number;
};

type Session = {
  socket: WebSocket;
  remoteAddress: string;
  isReady: boolean;
  /** Set when the server has decided to close, so a second failure does not send again. */
  isClosing: boolean;
  /** False once a ping has gone unanswered across one interval. */
  isAlive: boolean;
  helloTimer: ReturnType<typeof setTimeout> | undefined;
};

const CLOSE_REASON_LIMIT = 120;

/**
 * Starts the socket server.
 * @param options See {@link ChatSocketOptions}. The database is closed by {@link ChatSocketServer.close}.
 */
export function startChatSocketServer(options: ChatSocketOptions): Promise<ChatSocketServer> {
  const log = options.log;
  const now = options.now ?? (() => new Date());
  const helloTimeoutMs = options.helloTimeoutMs ?? HELLO_TIMEOUT_MS;
  const pingIntervalMs = options.pingIntervalMs ?? SERVER_PING_INTERVAL_MS;
  const attachments = createAttachmentReader(options.database);
  const messages = createMessageReader(options.database);
  const messageWriter = createMessageWriter(options.database);
  const sessions = new Set<Session>();

  const socketServer = new WebSocketServer({
    host: options.host ?? '::',
    port: options.port ?? DEFAULT_SOCKET_PORT,
    path: SOCKET_PATH,
    maxPayload: CHAT_LIMITS.maxFrameBytes,
    perMessageDeflate: false,
  });

  socketServer.on('connection', (socket: WebSocket, request: IncomingMessage) => {
    const remoteAddress = readRemoteAddress(request);
    // An oversized frame makes ws emit `error` after closing that socket. Without
    // this handler the error stops the whole process.
    socket.on('error', (error: Error) => {
      log('connection_error', { remoteAddress, code: errorCode(error) });
    });

    const session: Session = {
      socket,
      remoteAddress,
      isReady: false,
      isClosing: false,
      isAlive: true,
      helloTimer: undefined,
    };
    sessions.add(session);
    log('connection_opened', { remoteAddress });

    session.helloTimer = setTimeout(() => {
      if (session.isReady || session.isClosing) {
        return;
      }
      log('hello_refused', { remoteAddress, code: 'hello_timeout' });
      refuse(session, CLOSE_CODES.helloTimeout, 'hello_timeout', 'No hello arrived within 5 seconds.');
    }, helloTimeoutMs);

    socket.on('pong', () => {
      session.isAlive = true;
    });
    socket.on('close', (code: number) => {
      clearTimeout(session.helloTimer);
      sessions.delete(session);
      log('connection_closed', { remoteAddress, code });
    });
    socket.on('message', (data: RawData, isBinary: boolean) => {
      if (session.isClosing) {
        return;
      }
      try {
        handleFrame(session, data, isBinary);
      } catch (error) {
        log('connection_failed', { remoteAddress, code: errorCode(error) });
        refuse(session, CLOSE_CODES.internalError, 'internal_error', 'The server hit an unexpected error.');
      }
    });
  });

  const pingTimer = setInterval(() => {
    for (const session of [...sessions]) {
      if (session.socket.readyState !== WebSocket.OPEN) {
        continue;
      }
      if (!session.isAlive) {
        session.socket.terminate();
        continue;
      }
      session.isAlive = false;
      session.socket.ping();
    }
  }, pingIntervalMs);

  function handleFrame(session: Session, data: RawData, isBinary: boolean): void {
    if (isBinary) {
      if (!session.isReady) {
        log('hello_refused', { remoteAddress: session.remoteAddress, code: 'not_ready' });
        refuse(session, CLOSE_CODES.badFrame, 'not_ready', 'The first frame must be a hello.');
        return;
      }
      refuseFrame(session, 'Frames must be JSON text.');
      return;
    }
    const parsed = parseClientFrame(frameText(data));
    if (!session.isReady) {
      if (!parsed.ok || parsed.frame.type !== 'hello') {
        log('hello_refused', { remoteAddress: session.remoteAddress, code: 'not_ready' });
        refuse(session, CLOSE_CODES.badFrame, 'not_ready', 'The first frame must be a hello.');
        return;
      }
      handleHello(session, parsed.frame.protocol, parsed.frame.roomKey);
      return;
    }
    if (!parsed.ok) {
      if (parsed.error === 'invalid_message') {
        log('message_rejected', { id: parsed.messageId, code: 'invalid_message' });
        sendFrame(session.socket, { type: 'reject', id: parsed.messageId, code: 'invalid_message', detail: parsed.detail });
        return;
      }
      refuseFrame(session, parsed.detail);
      return;
    }
    switch (parsed.frame.type) {
      case 'hello':
        refuseFrame(session, 'The hello frame must be sent only once.');
        return;
      case 'ping':
        sendFrame(session.socket, { type: 'pong' });
        return;
      case 'send':
        handleSend(session, parsed.frame.message);
        return;
    }
  }

  function handleHello(session: Session, protocol: string, roomKey: string): void {
    clearTimeout(session.helloTimer);
    if (protocol !== PROTOCOL_VERSION) {
      log('hello_refused', { remoteAddress: session.remoteAddress, code: 'unsupported_protocol' });
      refuse(
        session,
        CLOSE_CODES.badFrame,
        'unsupported_protocol',
        `This server speaks ${PROTOCOL_VERSION}.`,
      );
      return;
    }
    if (!options.isRoomKey(roomKey)) {
      log('hello_refused', { remoteAddress: session.remoteAddress, code: 'unauthorized' });
      refuse(session, CLOSE_CODES.unauthorized, 'unauthorized', 'The room key is missing or wrong.');
      return;
    }
    session.isReady = true;
    sendFrame(session.socket, {
      type: 'welcome',
      protocol: PROTOCOL_VERSION,
      databaseId: options.database.databaseId,
      latestSeq: messages.readLatestSeq(MAIN_ROOM_ID),
      serverTime: now().toISOString(),
      limits: CHAT_LIMITS,
    });
  }

  function handleSend(session: Session, envelope: MessageEnvelope): void {
    const decision = reviewMessage(envelope, (attachmentId) => attachments.findAttachment(attachmentId));
    if (!decision.isAccepted) {
      log('message_rejected', { id: envelope.id, code: decision.code });
      sendFrame(session.socket, { type: 'reject', id: envelope.id, code: decision.code, detail: decision.detail });
      return;
    }
    let outcome: ReturnType<typeof messageWriter.storeMessage>;
    try {
      outcome = messageWriter.storeMessage(envelope, now().toISOString());
    } catch (error) {
      if (!isForeignKeyError(error)) {
        throw error;
      }
      log('message_rejected', { id: envelope.id, code: 'attachment_missing' });
      sendFrame(session.socket, {
        type: 'reject',
        id: envelope.id,
        code: 'attachment_missing',
        detail: 'no attachment is stored under attachmentId; upload it first',
      });
      return;
    }
    if (outcome.kind === 'conflict') {
      log('message_rejected', { id: envelope.id, code: 'id_conflict' });
      sendFrame(session.socket, {
        type: 'reject',
        id: envelope.id,
        code: 'id_conflict',
        detail: 'This message ID is already stored with different content.',
      });
      return;
    }
    const stored = outcome.message;
    log('message_stored', {
      id: stored.id,
      seq: stored.seq,
      senderId: stored.senderId,
      textLength: decision.summary.textLength,
      hasAttachment: decision.summary.hasAttachment,
      duplicate: outcome.kind === 'duplicate',
    });
    // Acknowledge only after the insert commits, and relay only after that.
    sendFrame(session.socket, {
      type: 'ack',
      id: stored.id,
      seq: stored.seq,
      serverReceivedAt: stored.serverReceivedAt,
      duplicate: outcome.kind === 'duplicate',
    });
    if (outcome.kind !== 'stored') {
      return;
    }
    const relay: ServerFrame = { type: 'message', message: stored };
    for (const other of sessions) {
      if (other !== session && other.isReady && !other.isClosing) {
        sendFrame(other.socket, relay);
      }
    }
  }

  function refuseFrame(session: Session, detail: string): void {
    log('frame_rejected', { remoteAddress: session.remoteAddress, code: 'bad_frame' });
    refuse(session, CLOSE_CODES.badFrame, 'bad_frame', detail);
  }

  let isClosed = false;
  function close(): Promise<void> {
    if (isClosed) {
      return Promise.resolve();
    }
    isClosed = true;
    clearInterval(pingTimer);
    for (const session of [...sessions]) {
      clearTimeout(session.helloTimer);
      if (session.socket.readyState === WebSocket.OPEN) {
        session.socket.close(CLOSE_CODES.serverShuttingDown, 'The server is shutting down.');
      }
    }
    return new Promise((resolve) => {
      socketServer.close(() => {
        if (options.database.connection.isOpen) {
          options.database.connection.close();
        }
        resolve();
      });
    });
  }

  return new Promise((resolve, reject) => {
    const failToListen = (error: Error) => {
      clearInterval(pingTimer);
      socketServer.close();
      reject(error);
    };
    socketServer.once('error', failToListen);
    socketServer.once('listening', () => {
      socketServer.off('error', failToListen);
      socketServer.on('error', (error: Error) => {
        log('server_failed', { code: errorCode(error) });
      });
      const address = socketServer.address();
      if (address === null || typeof address === 'string') {
        failToListen(new Error('The socket server has no port.'));
        return;
      }
      resolve({ port: address.port, close });
    });
  });
}

/**
 * Sends one frame. The socket's `error` handler covers a failed send.
 * @param socket An open connection.
 * @param frame A server frame.
 */
function sendFrame(socket: WebSocket, frame: ServerFrame): void {
  if (socket.readyState !== WebSocket.OPEN) {
    return;
  }
  socket.send(JSON.stringify(frame));
}

/**
 * Sends an error frame and then closes. The close waits until the frame is sent.
 * @param session The connection being refused.
 * @param closeCode A close code from section 7.3.
 * @param code The error frame's code.
 * @param detail A short explanation. It must not contain the room key or message text.
 */
function refuse(session: Session, closeCode: number, code: ConnectionErrorCode, detail: string): void {
  if (session.isClosing || session.socket.readyState !== WebSocket.OPEN) {
    return;
  }
  session.isClosing = true;
  clearTimeout(session.helloTimer);
  const frame: ErrorFrame = { type: 'error', code, detail };
  session.socket.send(JSON.stringify(frame), () => {
    if (session.socket.readyState === WebSocket.OPEN) {
      session.socket.close(closeCode, detail.slice(0, CLOSE_REASON_LIMIT));
    }
  });
}

function frameText(data: RawData): string {
  if (typeof data === 'string') {
    return data;
  }
  if (Array.isArray(data)) {
    return Buffer.concat(data).toString('utf8');
  }
  if (Buffer.isBuffer(data)) {
    return data.toString('utf8');
  }
  return Buffer.from(new Uint8Array(data)).toString('utf8');
}

function readRemoteAddress(request: IncomingMessage): string {
  const address = request.socket.remoteAddress;
  return typeof address === 'string' && address.length > 0 ? address : 'unknown';
}

/** A stable error code, never the message, which can quote stored text. */
function errorCode(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'code' in error && typeof error.code === 'string') {
    return error.code;
  }
  return 'unknown';
}

function isForeignKeyError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'message' in error &&
    typeof error.message === 'string' &&
    error.message.includes('FOREIGN KEY')
  );
}

async function main(): Promise<void> {
  const log = createJsonLogger((line) => {
    process.stdout.write(`${line}\n`);
  });
  const roomKey = readRoomKey(process.env);
  if (roomKey === null) {
    log('server_not_configured');
    process.stderr.write('The chat room key is not set. Run npm run chat:setup, then start the socket server again.\n');
    process.exitCode = 1;
    return;
  }
  const port = resolveSocketPort(process.env);
  if (!port.ok) {
    process.stderr.write(`${port.detail}\n`);
    process.exitCode = 1;
    return;
  }
  const database = openChatDatabase(resolveDatabasePath(process.env, process.cwd()));
  let server: ChatSocketServer;
  try {
    server = await startChatSocketServer({ database, isRoomKey: createRoomKeyCheck(roomKey), log, port: port.value });
  } catch (error) {
    log('server_failed', { code: errorCode(error) });
    if (database.connection.isOpen) {
      database.connection.close();
    }
    process.exitCode = 1;
    return;
  }
  log('server_listening', { port: server.port });

  let isShuttingDown = false;
  const shutdown = (): void => {
    if (isShuttingDown) {
      return;
    }
    isShuttingDown = true;
    server.close().then(
      () => {
        log('server_stopped');
        process.exit(0);
      },
      () => {
        log('server_failed', { code: 'shutdown' });
        process.exit(1);
      },
    );
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
}

if (import.meta.main) {
  main().catch(() => {
    process.stderr.write('The socket server failed to start.\n');
    process.exitCode = 1;
  });
}
