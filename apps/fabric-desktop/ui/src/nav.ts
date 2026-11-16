// The five screens and the hash navigation between them. One screen's section is visible; the
// current screen is location.hash, and an empty or unknown hash means the Review screen.
import { byId } from "./dom.ts";

export type ScreenId = "review" | "permissions" | "contacts" | "preferences" | "status";

export interface Screen {
  readonly id: ScreenId;
  readonly title: string; // the screen's h2 and the start of the window title
}

export const SCREENS: readonly Screen[] = [
  { id: "review", title: "Review agent actions" },
  { id: "permissions", title: "Agent permissions" },
  { id: "contacts", title: "Contacts" },
  { id: "preferences", title: "Calendar preferences" },
  { id: "status", title: "Status" },
];

export function screenFromHash(hash: string): ScreenId {
  return SCREENS.find((s) => `#${s.id}` === hash)?.id ?? "review";
}

export class Nav {
  private shown: ScreenId;
  private focusNext: (() => boolean) | null = null;
  private readonly onShow: (id: ScreenId) => void;

  // `onShow` runs whenever a screen becomes visible, before focus moves to it.
  constructor(onShow: (id: ScreenId) => void) {
    this.onShow = onShow;
    this.shown = screenFromHash(location.hash);
    this.show(this.shown); // the first load moves no focus
    window.addEventListener("hashchange", () => {
      this.shown = screenFromHash(location.hash);
      this.show(this.shown);
      this.focus(this.shown);
    });
  }

  get current(): ScreenId {
    return this.shown;
  }

  // Goes to a screen. `focus` may move focus somewhere inside it and says whether it did;
  // otherwise focus goes to the screen's heading.
  go(id: ScreenId, focus: (() => boolean) | null = null): void {
    if (location.hash === `#${id}`) {
      this.shown = id;
      this.show(id);
      if (!(focus?.() ?? false)) this.focus(id);
      return;
    }
    this.focusNext = focus;
    location.hash = id; // hashchange shows it
  }

  private show(id: ScreenId): void {
    for (const s of SCREENS) {
      const section = byId(s.id);
      const visible = s.id === id;
      if (section.hidden === visible) section.hidden = !visible;
      const link = byId(`nav-${s.id}`);
      if (visible) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    }
    const title = SCREENS.find((s) => s.id === id)?.title ?? "";
    if (document.title !== `${title} - Fabric Calendar`) document.title = `${title} - Fabric Calendar`;
    this.onShow(id);
  }

  private focus(id: ScreenId): void {
    const focus = this.focusNext;
    this.focusNext = null;
    if (!(focus?.() ?? false)) byId(`${id}-heading`).focus();
  }
}
