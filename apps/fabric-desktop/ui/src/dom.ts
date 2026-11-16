// Small helpers for the views. Nodes are made once and updated in place, so that focus and a
// screen reader's place survive updates. Text goes in with textContent only.
import type { Fact } from "./text.ts";

export function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (node === null) throw new Error(`index.html has no #${id}`);
  return node as T;
}

export function make<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className !== "") node.className = className;
  if (text !== "") node.textContent = text;
  return node;
}

// Changes the page only when the text differs, so a screen reader is not made to read it again.
export function setText(node: HTMLElement, text: string): void {
  if (node.textContent !== text) node.textContent = text;
}

// Hides or shows a node. A node that is hidden while it holds focus hands focus to `fallback`.
export function show(node: HTMLElement, visible: boolean, fallback: HTMLElement): void {
  if (!visible && !node.hidden && node.contains(document.activeElement)) fallback.focus();
  if (node.hidden !== !visible) node.hidden = !visible;
}

// A line that is shown only while it has text.
export function textLine(node: HTMLElement, text: string | null, fallback: HTMLElement): void {
  setText(node, text ?? "");
  show(node, text !== null && text !== "", fallback);
}

// aria-disabled, not disabled: the button keeps focus and stays in the tab order, and the text
// beside it (aria-describedby) says why it does nothing.
export function setDisabled(button: HTMLButtonElement, disabled: boolean): void {
  const value = String(disabled);
  if (button.getAttribute("aria-disabled") !== value) button.setAttribute("aria-disabled", value);
}

// A type="button" button whose press does nothing while it is aria-disabled.
export function button(text: string, onPress: () => void, className = ""): HTMLButtonElement {
  const node = make("button", className, text);
  node.type = "button";
  node.addEventListener("click", () => {
    if (node.getAttribute("aria-disabled") !== "true") onPress();
  });
  return node;
}

export function renderFacts(list: HTMLDListElement, facts: readonly Fact[]): void {
  list.replaceChildren();
  for (const f of facts) {
    list.append(make("dt", "", f.term));
    for (const v of f.values) {
      const dd = make("dd", f.untrusted === true ? "untrusted" : "");
      if (f.untrusted === true) dd.append(make("blockquote", "", v));
      else dd.textContent = v;
      list.append(dd);
    }
  }
}

const keyedNodes = new WeakMap<HTMLElement, Map<string, HTMLElement>>();

// Keeps one child of `container` per key, in the order of `items`: creates the missing ones,
// updates all, removes those whose key is gone (handing focus to `fallback` first when a removed
// node holds it). A node is never recreated, and is moved only when the order changed. The
// container holds nothing else.
export function keyed<T, N extends HTMLElement>(
  container: HTMLElement,
  items: readonly T[],
  keyOf: (item: T) => string,
  create: (item: T) => N,
  update: (node: N, item: T) => void,
  fallback: HTMLElement,
): void {
  const before = (keyedNodes.get(container) ?? new Map()) as Map<string, N>;
  const after = new Map<string, N>();
  for (const item of items) {
    const key = keyOf(item);
    if (after.has(key)) continue;
    const node = before.get(key) ?? create(item);
    after.set(key, node);
    update(node, item);
  }
  for (const [key, node] of before) {
    if (after.has(key)) continue;
    if (node.contains(document.activeElement)) fallback.focus();
    node.remove();
  }
  let i = 0;
  for (const node of after.values()) {
    const at = container.children[i] ?? null;
    if (at !== node) container.insertBefore(node, at);
    i += 1;
  }
  keyedNodes.set(container, after);
}

let uids = 0;

// A unique id for aria-describedby, aria-controls or a label's `for`.
export function uid(prefix: string): string {
  uids += 1;
  return `${prefix}-${uids}`;
}
