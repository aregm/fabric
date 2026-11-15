import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { StoredMessage } from '../shared/protocol.ts';
import type { OutboxItem } from './outbox.ts';
import { createChatState, highestHeldSeq, reduceChat, selectVisibleMessages } from './state.ts';

const TIME = '2026-10-26T05:00:00.000Z';

function stored(seq: number, senderId = 'alice', text = `m${seq}`): StoredMessage {
  return {
    id: `00000000-0000-4000-8000-00000000000${seq}`,
    roomId: 'main',
    senderId,
    clientCreatedAt: TIME,
    attachmentId: null,
    payload: { format: 'plain/1', text, attachment: null },
    seq,
    serverReceivedAt: TIME,
  };
}

function outbox(id: string, state: OutboxItem['state'], created = TIME): OutboxItem {
  return {
    id,
    senderId: 'alice',
    clientCreatedAt: created,
    text: 'draft',
    attachment: null,
    state,
    failureCode: state === 'failed' ? 'ack_timeout' : null,
    attempts: 1,
  };
}

describe('reduceChat', () => {
  it('merges history, live messages and acks by ID, and drops outbox copies', () => {
    let state = createChatState();
    const first = stored(1);
    const second = stored(2, 'bob');
    state = reduceChat(state, {
      type: 'history_page',
      databaseId: 'db-1',
      messages: [first],
      latestSeq: 2,
      hasMore: true,
      mode: 'first',
    });
    state = reduceChat(state, { type: 'outbox_changed', items: [outbox(second.id, 'sending')] });
    state = reduceChat(state, { type: 'live_message', message: second });
    assert.equal(state.messages[second.id]?.seq, 2);
    assert.equal(state.outbox.length, 0);
    assert.equal(state.hasOlder, true);
    assert.deepEqual(
      selectVisibleMessages(state).map((item) => item.key),
      [first.id, second.id],
    );
  });

  it('builds a sent message from an ack when history has not arrived yet', () => {
    const item = outbox('00000000-0000-4000-8000-000000000099', 'sending');
    let state = reduceChat(createChatState(), { type: 'outbox_changed', items: [item] });
    state = reduceChat(state, { type: 'acked', id: item.id, seq: 7, serverReceivedAt: TIME });
    assert.equal(state.messages[item.id]?.seq, 7);
    assert.equal(state.outbox.length, 0);
    assert.equal(highestHeldSeq(state), 7);
  });

  it('clears history when the database ID changes, and keeps the outbox', () => {
    const waiting = outbox('00000000-0000-4000-8000-000000000088', 'waiting');
    let state = reduceChat(createChatState(), {
      type: 'history_page',
      databaseId: 'db-1',
      messages: [stored(1)],
      latestSeq: 1,
      hasMore: false,
      mode: 'first',
    });
    state = reduceChat(state, { type: 'outbox_changed', items: [waiting] });
    state = reduceChat(state, {
      type: 'history_page',
      databaseId: 'db-2',
      messages: [stored(1, 'bob', 'new')],
      latestSeq: 1,
      hasMore: false,
      mode: 'first',
    });
    assert.equal(state.databaseId, 'db-2');
    assert.equal(state.messages[stored(1).id]?.senderId, 'bob');
    assert.equal(state.outbox[0]?.id, waiting.id);
  });

  it('puts unsent messages after sent ones, in device time order', () => {
    let state = reduceChat(createChatState(), {
      type: 'history_page',
      databaseId: 'db-1',
      messages: [stored(2), stored(1)],
      latestSeq: 2,
      hasMore: false,
      mode: 'first',
    });
    state = reduceChat(state, {
      type: 'outbox_changed',
      items: [
        outbox('00000000-0000-4000-8000-0000000000aa', 'waiting', '2026-10-26T05:00:02.000Z'),
        outbox('00000000-0000-4000-8000-0000000000bb', 'failed', '2026-10-26T05:00:01.000Z'),
      ],
    });
    assert.deepEqual(
      selectVisibleMessages(state).map((item) => item.key),
      [stored(1).id, stored(2).id, '00000000-0000-4000-8000-0000000000bb', '00000000-0000-4000-8000-0000000000aa'],
    );
  });
});
