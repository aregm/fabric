import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { DEMO_USERS, findDemoUser, isDemoUserId } from './users.ts';

describe('demo users', () => {
  it('are Alice and Bob', () => {
    assert.deepEqual(
      DEMO_USERS.map((user) => [user.id, user.displayName]),
      [
        ['alice', 'Alice'],
        ['bob', 'Bob'],
      ],
    );
  });

  it('are recognized only by their exact IDs', () => {
    assert.equal(isDemoUserId('alice'), true);
    assert.equal(isDemoUserId('bob'), true);
    for (const value of ['Alice', 'carol', '', undefined, null]) {
      assert.equal(isDemoUserId(value), false, String(value));
    }
  });

  it('can be found by ID', () => {
    assert.equal(findDemoUser('bob')?.displayName, 'Bob');
    assert.equal(findDemoUser('carol'), undefined);
  });
});
