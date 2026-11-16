// Wiring: the Tauri host, the real clock and ids, the model and the view.
import { tauriHost } from "./host.ts";
import { Model } from "./model.ts";
import { localZone } from "./text.ts";
import { View } from "./view.ts";

const host = tauriHost();
if (host === null) {
  const problem = document.getElementById("problem");
  if (problem !== null) {
    problem.textContent = "This page works only inside the Fabric Calendar app.";
    problem.hidden = false;
  }
} else {
  let view: View | null = null;
  const model = new Model({
    host,
    now: () => Date.now(),
    newId: () => crypto.randomUUID(),
    zone: localZone(),
    changed: () => view?.render(),
    announce: (text) => view?.announce(text),
  });
  view = new View(model);
  view.render();
  model.start();
}
