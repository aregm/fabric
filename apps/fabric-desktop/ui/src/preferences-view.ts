// Placeholder for the screen in #preferences; it is replaced as a whole.
import { byId, make } from "./dom.ts";
import type { Model } from "./model.ts";

export class PreferencesView {
  private readonly model: Model;

  constructor(model: Model) {
    this.model = model;
    byId("preferences").append(make("p", "note", "The calendar preferences screen comes in a later step of this build."));
  }

  render(): void {
    void this.model;
  }
}
