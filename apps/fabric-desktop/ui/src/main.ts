// Wiring: the Tauri host, the real clock and ids, the model, the navigation, the shell and the
// screens. After every change the shell and the visible screen render; a screen also renders
// when it becomes visible.
import { byId, make } from "./dom.ts";
import { GroupsView } from "./groups-view.ts";
import { tauriHost } from "./host.ts";
import { Model } from "./model.ts";
import { Nav } from "./nav.ts";
import type { ScreenId } from "./nav.ts";
import { PreferencesView } from "./preferences-view.ts";
import { ProfilesView } from "./profiles-view.ts";
import { ReviewView } from "./review-view.ts";
import { Shell } from "./shell.ts";
import { localZone } from "./text.ts";

const host = tauriHost();
if (host === null) {
  const strip = byId("attention");
  byId("attention-list").replaceWith(make("p", "problem", "This page works only inside the Fabric Calendar app."));
  strip.hidden = false;
} else {
  let shell: Shell | null = null;
  let render = (): void => {};
  const model = new Model({
    host,
    now: () => Date.now(),
    newId: () => crypto.randomUUID(),
    zone: localZone(),
    changed: () => render(),
    announce: (text) => shell?.announce(text),
  });
  const review = new ReviewView(model);
  const screens: Record<ScreenId, { render(): void } | null> = {
    review,
    permissions: new ProfilesView(model),
    contacts: new GroupsView(model),
    preferences: new PreferencesView(model),
    status: null, // the shell renders the Status screen
  };
  const nav = new Nav((id) => screens[id]?.render());
  shell = new Shell(model, nav, review);
  render = () => {
    shell?.render();
    screens[nav.current]?.render();
  };
  render();
  model.start();
}
