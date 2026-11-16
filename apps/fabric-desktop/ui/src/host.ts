// The host bridge (design note section 4): the three Tauri commands and the messages the host
// sends on the channel. This is the only file that touches `window.__TAURI__`, which exists
// because the app sets `withGlobalTauri`. The bridge is Tauri IPC, not part of fabric-desktop/1.
import type { ErrorCode, OwnerNotice, OwnerResponse } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";

export type HostState =
  | { readonly name: "connected" }
  | { readonly name: "disconnected"; readonly reason: "exited" | "protocol_violation" | "stopped" | "start_failed"; readonly detail: string }
  | { readonly name: "conflict"; readonly detail: string }
  | { readonly name: "configuration_error"; readonly detail: string };

// `run` counts broker starts: 0 before any, 1 for the first.
export type HostMessage =
  | { readonly kind: "state"; readonly run: number; readonly state: HostState }
  | { readonly kind: "frame"; readonly run: number; readonly frame: OwnerResponse | OwnerNotice };

export type SendRefusal =
  | { readonly kind: "refused"; readonly layer: "frame" | "protocol" | "schema"; readonly code: ErrorCode; readonly reason: string }
  | { readonly kind: "not_connected" }
  | { readonly kind: "io"; readonly detail: string };

export interface Host {
  // Sends the current state at once, then every state change and every frame.
  attach(listener: (message: HostMessage) => void): Promise<void>;
  // One owner request frame as JSON text. Rejects with a SendRefusal.
  send(frame: string): Promise<void>;
  // Allowed only while disconnected. Rejects with a string.
  restart(): Promise<void>;
}

interface TauriChannel<T> {
  onmessage: (message: T) => void;
}
interface TauriCore {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
  readonly Channel: new <T>() => TauriChannel<T>;
}
declare global {
  interface Window {
    readonly __TAURI__?: { readonly core: TauriCore };
  }
}

// Null when the page is not running inside the app (or the harness).
export function tauriHost(): Host | null {
  const core = window.__TAURI__?.core;
  if (core === undefined) return null;
  return {
    async attach(listener) {
      const channel = new core.Channel<HostMessage>();
      channel.onmessage = listener;
      await core.invoke("owner_attach", { channel });
    },
    async send(frame) {
      await core.invoke("owner_send", { frame });
    },
    async restart() {
      await core.invoke("broker_restart");
    },
  };
}
