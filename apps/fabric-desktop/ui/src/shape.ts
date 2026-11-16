// Shape checks of what the owner types, by the contract's patterns and bounds
// (contracts/desktop/v1/schemas/common.schema.json). They decide nothing: they let the page name a
// mistake at the field, so that the host never has to refuse a frame. Whether a zone exists is
// the broker's to say.

const EMAIL = /^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]{1,64}@[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+$/u;
const DOMAIN = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+$/u;
const ZONE = /^[A-Za-z][A-Za-z0-9_+-]*(\/[A-Za-z0-9_+-]+)*$/u;
const LOCAL_TIME = /^([01][0-9]|2[0-3]):[0-5][0-9]$/u;

// Bounds count Unicode code points.
const length = (text: string): number => [...text].length;

// Only ASCII letters are lowered: the contract's patterns allow nothing else.
const asciiLower = (text: string): string => text.replace(/[A-Z]/g, (c) => c.toLowerCase());

// A label: 1 to 200 code points, as typed.
export function isLabel(text: string): boolean {
  const n = length(text);
  return n >= 1 && n <= 200;
}

// An address in normal form: surrounding spaces dropped, the part before the @ exactly as typed,
// the domain in lower case. Null when it is not an address by the contract's pattern.
export function normalAddress(text: string): string | null {
  const trimmed = text.trim();
  const at = trimmed.indexOf("@");
  if (at < 0) return null;
  const normal = trimmed.slice(0, at + 1) + asciiLower(trimmed.slice(at + 1));
  return length(normal) <= 320 && EMAIL.test(normal) ? normal : null;
}

// A domain in normal form: surrounding spaces dropped, lower case. Null when it is not a domain.
export function normalDomain(text: string): string | null {
  const normal = asciiLower(text.trim());
  return length(normal) <= 253 && DOMAIN.test(normal) ? normal : null;
}

// The shape of an IANA zone name.
export function isZone(text: string): boolean {
  return length(text) <= 64 && ZONE.test(text);
}

// A time of day to the minute, "09:30".
export function isLocalTime(text: string): boolean {
  return LOCAL_TIME.test(text);
}
