// Placeholder for the screen in #permissions; it is replaced as a whole.
import { byId, make } from "./dom.ts";
import type { Model } from "./model.ts";

export class ProfilesView {
  private readonly model: Model;

  constructor(model: Model) {
    this.model = model;
    byId("permissions").append(make("p", "note", "The agent permissions screen comes in a later step of this build."));
  }

  render(): void {
    void this.model;
  }
}
