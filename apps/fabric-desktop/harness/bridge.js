// Development harness only, never shipped. Defines the minimal window.__TAURI__.core that
// ui/src/host.ts uses (invoke and Channel) over fetch and EventSource, so that the built page
// can run in an ordinary browser against harness/serve.ts.
"use strict";
(() => {
  class Channel {
    onmessage = () => {};
  }

  let events = null;

  async function post(path, payload) {
    const response = await fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: payload === undefined ? undefined : JSON.stringify(payload),
    });
    if (response.ok) return null;
    const text = await response.text();
    throw text === "" ? `HTTP ${response.status}` : JSON.parse(text);
  }

  async function invoke(command, args) {
    switch (command) {
      case "owner_attach": {
        events?.close(); // a new attach replaces the channel
        events = new EventSource("/harness/events");
        const channel = args.channel;
        events.onmessage = (event) => channel.onmessage(JSON.parse(event.data));
        await new Promise((resolve, reject) => {
          events.onopen = resolve;
          events.onerror = () => reject("The harness event stream could not be opened.");
        });
        events.onerror = null; // EventSource reconnects by itself, and the server then sends the state again
        return null;
      }
      case "owner_send":
        return post("/harness/owner_send", { frame: args.frame });
      case "broker_restart":
        return post("/harness/broker_restart");
      default:
        throw `The harness does not implement ${command}.`;
    }
  }

  window.__TAURI__ = { core: { invoke, Channel } };
})();
