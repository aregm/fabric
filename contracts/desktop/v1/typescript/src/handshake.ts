// The agent handshake text and the pairing code (README section 8). Signing and verifying
// belong to the agent adapter and the broker; the tests verify the fixtures with node:crypto.
import { canonical, sha256Hex } from "./canonical.ts";
import { PROTOCOL } from "./contract.ts";

// The text an agent signs: two lines, then the canonical form of the four handshake values.
export function handshakeText(schemaVersion: number, enrollmentId: string, publicKey: string, brokerNonce: string): string {
  const subject = {
    broker_nonce: brokerNonce,
    enrollment_id: enrollmentId,
    public_key: publicKey,
    schema_version: schemaVersion,
  };
  return `${PROTOCOL}\nagent_handshake\n${canonical(subject)}`;
}

// The first twelve hexadecimal digits, in capitals, of the key's SHA-256, written XXXX-XXXX-XXXX.
export function pairingCode(publicKey: string): string {
  const code = sha256Hex(`${PROTOCOL}\nagent_key\n${publicKey}`).slice(0, 12).toUpperCase();
  return `${code.slice(0, 4)}-${code.slice(4, 8)}-${code.slice(8, 12)}`;
}
