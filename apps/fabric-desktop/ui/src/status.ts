// What needs the owner now, and the facts of the Status screen, in words. Pure: no DOM.
import type { Calendar } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Model } from "./model.ts";
import { ACCESS_TEXT, ACCOUNT_TEXT, PROVER_TEXT, STORE_PROBLEM, STORE_TEXT, bannerText, formatTimeOfDay } from "./text.ts";

export interface AttentionItem {
  readonly key: string;
  readonly kind: "problem" | "notice";
  readonly title: string;
  readonly text: string;
  readonly detail: string | null;
  readonly action: "restart" | "retry" | "review" | null;
}

const NO_ACCOUNT_CONNECT = "This build cannot connect or reconnect a Google account yet.";
const ACCOUNT_NEEDS_OWNER = new Set(["reauthentication_required", "consent_denied", "partial_grant", "account_mismatch", "cancelled"]);

// Health is what the broker of this run said; it is not shown for an earlier run.
function currentHealth(m: Model) {
  return m.connection?.name === "connected" && m.healthRun === m.run ? m.health : null;
}

// In this order: the broker, the sync, the store, the rule checker, the accounts, waiting requests.
export function attention(m: Model): AttentionItem[] {
  const items: AttentionItem[] = [];
  const state = m.connection;
  const banner = bannerText(state);
  if (banner !== null && state !== null && state.name !== "connected") {
    items.push({
      key: "broker", kind: "problem", title: banner.title, text: banner.text, detail: `Details: ${state.detail}`,
      action: state.name === "disconnected" ? "restart" : null,
    });
  }
  const f = m.syncFailure;
  if (m.phase === "failed" && f !== null) {
    items.push(f.error?.code === "store_unavailable"
      ? {
        key: "sync", kind: "problem", title: STORE_PROBLEM[f.error.detail.reason],
        text: `${f.text} Nothing is read or written until the store opens.`, detail: null, action: "retry",
      }
      : { key: "sync", kind: "problem", title: "The current state could not be read from the broker.", text: f.text, detail: null, action: "retry" });
  }
  const health = currentHealth(m);
  if (health !== null && health.store !== "ready") {
    items.push({
      key: "store", kind: "problem", title: STORE_PROBLEM[health.store], text: "Nothing is read or written until the store opens.",
      detail: null, action: null,
    });
  }
  if (health?.prover === "unavailable") {
    items.push({
      key: "prover", kind: "notice", title: "The rule checker is not running.",
      text: "Fabric allows nothing it cannot prove. Until the rule checker runs, an agent's request ends as not done, also after you allow it.",
      detail: null, action: null,
    });
  } else if (health?.prover === "starting") {
    items.push({
      key: "prover", kind: "notice", title: "The rule checker is starting.", text: "Requests wait until it is ready.", detail: null, action: null,
    });
  }
  for (const a of m.accounts.values()) {
    if (!ACCOUNT_NEEDS_OWNER.has(a.state)) continue;
    items.push({
      key: `account-${a.account_id}`, kind: "problem", title: `${a.email ?? "An account"}: ${ACCOUNT_TEXT[a.state]}.`,
      text: NO_ACCOUNT_CONNECT, detail: a.email === null ? `Account ${a.account_id}` : null, action: null,
    });
  }
  let waiting = 0;
  for (const e of m.requests.values()) if (e.available && e.request.status.state === "pending_approval") waiting += 1;
  if (waiting > 0) {
    items.push({
      key: "waiting", kind: "notice",
      title: waiting === 1 ? "1 request waits for your decision." : `${waiting} requests wait for your decision.`,
      text: "", detail: null, action: "review",
    });
  }
  return items;
}

// The connection in words, for the Status screen.
export function connectionText(m: Model): string {
  const state = m.connection;
  if (state === null) return "Connecting to the broker…";
  if (state.name !== "connected") return state.name === "disconnected" ? "Stopped" : "Not started";
  if (m.phase === "live") return "Running and up to date";
  if (m.phase === "failed") return "Running, but the current state could not be read";
  return "Running; reading the current state…";
}

export function brokerText(m: Model): string {
  return m.broker === null ? "Not known yet" : `${m.broker.broker.name}, build ${m.broker.broker.build}`;
}

export function lastCheckText(m: Model): string {
  return m.lastCheck === null ? "None yet" : `Answered in ${m.lastCheck.ms} ms, at ${formatTimeOfDay(m.lastCheck.at, m.zone)}`;
}

// The header's build line. It claims nothing about what the broker can do.
export function buildLine(m: Model): string {
  return m.broker === null ? "Development build." : `Development build. Broker: ${brokerText(m)}.`;
}

export function storeText(m: Model): string {
  const h = currentHealth(m);
  return h === null ? "Not known while the broker's current state has not been read" : STORE_TEXT[h.store];
}

export function proverText(m: Model): string {
  const h = currentHealth(m);
  return h === null ? "Not known while the broker's current state has not been read" : PROVER_TEXT[h.prover];
}

// One calendar of an account on the Status screen.
export function calendarLine(c: Calendar): string {
  const parts = [`${c.name}: ${ACCESS_TEXT[c.access]}`];
  if (c.selected) parts.push("agents may use it");
  if (c.agent_drafts) parts.push("Agent Drafts");
  return parts.join("; ");
}
