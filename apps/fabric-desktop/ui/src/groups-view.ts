// Placeholder for the screen in #contacts; it is replaced as a whole.
import { byId, make } from "./dom.ts";
import type { Model } from "./model.ts";

export class GroupsView {
  private readonly model: Model;

  constructor(model: Model) {
    this.model = model;
    byId("contacts").append(make("p", "note", "The contacts screen comes in a later step of this build."));
  }

  render(): void {
    void this.model;
  }
}
