/**
 * The fixed demo users (D16). Identities are self-asserted: anyone with the
 * room key can post as either user (docs/paracels-chat.md, section 13).
 */

export type DemoUserId = 'alice' | 'bob';

export type DemoUser = {
  /** Sent as `senderId` and in the `X-Chat-Uploader` header. */
  id: DemoUserId;
  /** The name the app shows. */
  displayName: string;
};

export const DEMO_USERS: readonly Readonly<DemoUser>[] = Object.freeze([
  Object.freeze({ id: 'alice', displayName: 'Alice' }),
  Object.freeze({ id: 'bob', displayName: 'Bob' }),
]);

/**
 * Tells whether a value is the ID of a demo user.
 * @param value Any value, typically a `senderId` from the network.
 */
export function isDemoUserId(value: unknown): value is DemoUserId {
  return DEMO_USERS.some((user) => user.id === value);
}

/**
 * Finds a demo user by ID.
 * @param userId The ID to look up.
 * @returns The user, or `undefined` for an unknown ID.
 */
export function findDemoUser(userId: string): Readonly<DemoUser> | undefined {
  return DEMO_USERS.find((user) => user.id === userId);
}
