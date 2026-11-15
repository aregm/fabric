// The demo dashboard (section 13). Plain JS, no build step, no external requests.
// Every value from the services is inserted as text, never as HTML: peer notes are untrusted.
"use strict";

const key = new URLSearchParams(location.hash.slice(1)).get("key") || "";
const state = { overview: null, events: [], lastSeq: 0, counts: {} };

function h(tag, props = {}, ...children) {
  const element = document.createElement(tag);
  for (const [name, value] of Object.entries(props)) {
    if (name === "class") element.className = value;
    else if (name.startsWith("on")) element.addEventListener(name.slice(2), value);
    else if (value !== undefined && value !== null && value !== false) element.setAttribute(name, value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    element.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return element;
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { "x-dashboard-key": key, "content-type": "application/json" },
  });
  return response.json();
}

function utc(ms) {
  if (ms === undefined || ms === null) return "—";
  return new Date(ms).toISOString().replace(".000Z", "Z").replace("T", " ");
}

function friendly(ms, minutes) {
  const start = new Date(ms);
  const end = new Date(ms + minutes * 60000);
  const day = start.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
  const time = (d) => d.toISOString().slice(11, 16);
  return `${day}, ${time(start)}–${time(end)} UTC`;
}

function pill(text, kind) {
  return h("span", { class: `pill ${kind || ""}` }, text);
}

function verdictPill(verdict) {
  if (!verdict) return pill("unchecked");
  if (verdict.verdict === "permit") return pill("permitted", "permit");
  if (verdict.verdict === "deny") return pill("denied", "deny");
  return pill("undecided", "wait");
}

function verdictReasons(verdict) {
  if (!verdict) return [];
  if (verdict.verdict === "deny") return verdict.denials.map((d) => `${d.rule}: ${d.detail}`);
  if (verdict.verdict === "undecided") return [`undecided: ${verdict.reason}`];
  return [];
}

function describe(action) {
  if (!action) return "";
  const parts = Object.entries(action)
    .filter(([name]) => name !== "kind" && name !== "negotiation")
    .map(([name, value]) => {
      if (name.endsWith("_ms") && typeof value === "number") {
        const label = name.slice(0, -3);
        return ["length", "duration"].includes(label) ? `${label}=${value / 60000} min` : `${label}=${utc(value)}`;
      }
      if (Array.isArray(value)) return `${name}=[${value.length}]`;
      return `${name}=${JSON.stringify(value)}`;
    });
  return `${action.kind}(${parts.join(", ")})`;
}

function proofLink(id) {
  if (!id) return null;
  return h("a", { class: "proof", onclick: () => showProof(id) }, `proof ${id.slice(0, 10)}…`);
}

async function showProof(id) {
  const report = await api(`/v1/proof?id=${encodeURIComponent(id)}`);
  const body = document.getElementById("proof-body");
  body.replaceChildren();
  document.getElementById("proof-title").textContent = report.theory ? `${report.theory} (${report.kind})` : "Proof";
  if (report.error) {
    body.append(h("p", { class: "empty" }, report.detail || report.error));
  } else {
    body.append(
      h("p", {}, `Isabelle ${report.isabelle}, ${report.seconds.toFixed(2)} s. Proof ID ${report.proof_id}.`),
      ...report.verdicts.map((v) =>
        h("div", { class: "plan" },
          h("div", { class: "action" }, describe(v.action), verdictPill(v.verdict)),
          ...verdictReasons(v.verdict).map((r) => h("div", { class: "reason" }, r)),
          h("div", { class: "rationale" }, v.rules.map((r) => `${r.rule}: ${r.outcome}`).join(" · ")),
        )),
      h("h3", {}, "Check theory"),
      h("pre", {}, report.theory_text),
    );
  }
  document.getElementById("proof").showModal();
}

function records(source, kind) {
  return state.events
    .filter((e) => e.item.source === source && (!kind || e.item.record.kind === kind))
    .map((e) => e.item.record);
}

// ----- one person's column -----

function requestCard(person) {
  const status = person.status || {};
  if (status.request) {
    const r = status.request.request;
    return h("div", {},
      h("h3", {}, "Owner's request"),
      h("p", {}, `${r.duration_ms / 60000} minutes with ${r.peer}, between ${utc(r.window_start_ms)} and ${utc(r.window_end_ms)}. The negotiation expires at ${utc(r.expires_at_ms)}.`),
      status.request.preference ? h("p", { class: "hint" }, `Preference (only this owner's agent reads it): ${status.request.preference}`) : null,
    );
  }
  const form = person.request_form;
  const field = (name, label, value, full) =>
    h("label", { class: full ? "full" : "" }, label, h("input", { name, value }));
  const element = h("form", { class: "form", onsubmit: async (event) => {
    event.preventDefault();
    await fileRequest(person.name, element);
    refresh();
  } },
    field("peer", "Meet with", form.peer),
    field("duration_minutes", "Minutes", form.duration_minutes),
    field("window_start", "Window start (UTC)", form.window_start),
    field("window_end", "Window end (UTC)", form.window_end),
    field("expires_at", "Negotiation expires (UTC)", form.expires_at, true),
    h("label", { class: "full" }, "Preference (only this owner's agent reads it)", h("textarea", { name: "preference", rows: 2 }, form.preference)),
    h("div", { class: "full" }, h("button", { class: "primary", type: "submit" }, "File request")),
  );
  return h("div", {}, h("h3", {}, "Owner's request"), element);
}

// Files the owner's request as the form shows it, and shows the host's refusal under the form.
async function fileRequest(name, form) {
  const data = Object.fromEntries(new FormData(form));
  data.duration_minutes = Number(data.duration_minutes);
  const answer = await api(`/v1/people/${name}/request`, { method: "POST", body: JSON.stringify(data) });
  form.querySelector(".reason")?.remove();
  if (answer.error) form.append(h("p", { class: "reason full" }, `Refused: ${answer.error}. ${answer.detail || ""}`));
}

function calendarCard(person) {
  const events = (person.events || []).slice().sort((a, b) => a.start_ms - b.start_ms);
  return h("div", {},
    h("h3", {}, "Calendar (owner's view, with titles)"),
    events.length === 0
      ? h("p", { class: "empty" }, person.calendar_error ? `Calendar unavailable: ${person.calendar_error}` : "No events")
      : h("ul", { class: "events" }, events.map((e) =>
          h("li", {},
            h("span", { class: "when" }, `${utc(e.start_ms)} · ${e.length_ms / 60000} min`),
            h("span", { class: e.added_by ? "added" : e.busy ? "" : "free" }, e.title, e.busy ? "" : " (free)"),
            e.added_by ? proofLink(e.added_by.proof) : null,
          ))),
  );
}

function decisionsCard(person) {
  const status = person.status || {};
  const negotiation = status.negotiation;
  if (!negotiation || negotiation.mutual.length === 0) {
    return h("div", {}, h("h3", {}, "Owner's decisions"), h("p", { class: "empty" }, "No mutual options yet"));
  }
  const peer = status.peer;
  const sent = negotiation.sent_decisions.length > 0;
  const answers = Object.fromEntries((status.owner_decisions || []).map((d) => [d.candidate_id, d.accept]));
  const expires = status.request ? utc(status.request.request.expires_at_ms) : "—";
  return h("div", {},
    h("h3", {}, "Owner's decisions"),
    negotiation.mutual.map((id) => {
      const candidate = negotiation.candidates.find((c) => c.id === id);
      const when = candidate ? friendly(candidate.start_ms, candidate.length_ms / 60000) : id;
      const answer = answers[id];
      const decide = (yes) => async () => {
        await api(`/v1/people/${person.name}/consent`, { method: "POST", body: JSON.stringify({ candidate_id: id, yes }) });
        refresh();
      };
      return h("div", { class: "option" },
        h("p", {}, `Yes lets your agent add this exact meeting: ${when} with ${peer}. This consent expires at ${expires}.`),
        h("div", { class: "buttons" },
          h("button", { class: answer === true ? "primary" : "", disabled: sent, onclick: decide(true) }, "Yes"),
          h("button", { class: answer === false ? "primary" : "", disabled: sent, onclick: decide(false) }, "No"),
          answer === undefined ? pill("no answer yet", "wait") : pill(answer ? "you said yes" : "you said no", answer ? "permit" : "deny"),
          sent ? pill("sent") : null,
        ));
    }),
  );
}

function agentCard(person) {
  const source = `${person.name}-agent`;
  const turns = records(source, "plans");
  const latest = turns[turns.length - 1];
  const executed = records(source, "executed").slice(-8).reverse();
  const authorized = records(`${person.name}-host`, "authorize").filter((r) => r.token).slice(-6).reverse();
  const reads = executed.filter((r) => r.action.kind === "read_free_busy");
  const status = person.status || {};
  const negotiation = status.negotiation || {};

  const plans = [];
  if (latest) {
    const verdicts = (latest.screen && latest.screen.verdicts) || [];
    let position = 0;
    (latest.options || []).forEach((option, index) => {
      const label = latest.plans.plans[index - ((latest.options.length - latest.plans.plans.length))] || {};
      if (option.unreadable) {
        plans.push(h("div", { class: "plan" }, h("div", { class: "reason" }, `unreadable plan: ${option.unreadable}`)));
        return;
      }
      const actions = option.actions.map((action) => {
        const verdict = verdicts[position++];
        return h("div", {},
          h("div", { class: "action" }, describe(action), verdictPill(verdict)),
          ...verdictReasons(verdict).map((r) => h("div", { class: "reason" }, r)));
      });
      plans.push(h("div", { class: `plan ${latest.chosen === index ? "chosen" : ""}` },
        h("div", { class: "label" }, label.label || "(added by a fault switch)", latest.chosen === index ? " — runs" : ""),
        label.rationale ? h("div", { class: "rationale" }, label.rationale) : null,
        actions.length ? actions : h("div", { class: "rationale" }, "wait for the other person or the owner")));
    });
  }
  const planner = latest && latest.planner;
  return h("div", {},
    h("h3", {}, "Agent"),
    h("p", {},
      `Session: ${status.link || "—"} · negotiation ${negotiation.id || "—"} · round ${negotiation.round ?? "—"}`,
      negotiation.closed ? h("span", {}, " · ", pill(`closed: ${negotiation.closed}`, "permit")) : null),
    latest
      ? h("div", {},
          h("p", { class: "hint" }, `Turn ${latest.turn}, attempt ${latest.attempt + 1}. ${planner.model}: ${planner.input_tokens} in, ${planner.output_tokens} out, ${planner.seconds.toFixed(1)} s.`,
            latest.screen ? h("span", {}, " Screened by ", proofLink(latest.screen.proof_id), ` in ${latest.screen.seconds.toFixed(2)} s.`) : null),
          plans)
      : h("p", { class: "empty" }, "No turn yet"),
    h("h3", {}, "Executed actions and their tokens"),
    authorized.length === 0
      ? h("p", { class: "empty" }, "None yet")
      : h("div", { class: "table-wrap" }, h("table", {},
          h("thead", {}, h("tr", {}, h("th", {}, "Action"), h("th", {}, "Token"), h("th", {}, "Proof"))),
          h("tbody", {}, authorized.map((r) => h("tr", {},
            h("td", { class: "mono" }, r.token.action),
            h("td", { class: "mono" }, `${r.token.audience}, nonce ${r.token.nonce.slice(0, 8)}…, expires ${utc(r.token.expires_at * 1000)}`),
            h("td", {}, proofLink(r.proof_id))))))),
    h("h3", {}, "Free/busy the agent received"),
    reads.length === 0
      ? h("p", { class: "empty" }, "Nothing read yet")
      : h("p", { class: "mono" }, (reads[0].result.busy || []).map((b) => `${utc(b.start_ms).slice(5, 16)}–${utc(b.end_ms).slice(11, 16)}`).join(", ") || "no busy time"),
  );
}

const FAULTS = [
  ["over_broad_read", "Over-broad read",
    "Next turn: adds a read of event details and widens the free/busy read. Isabelle denies both when screening."],
  ["too_many_candidates", "Too many candidates",
    "Next proposal (only the coordinator, Alice, proposes): pads it to 5 candidates. Isabelle denies it when screening."],
  ["write_without_consent", "Write without consent",
    "Next plan that acts: puts an add_meeting first. Isabelle denies it when authorizing."],
  ["replay_token", "Replay a token",
    "After the next executed action: sends it again with the same token. The service refuses it as replayed."],
  ["token_for_different_slot", "Token for a different slot",
    "Next calendar action: tries its token on a slot 15 minutes later. The service refuses the mismatch."],
];

// What happened to each switch: armed by the owner, then fired by the agent.
function faultStates(person) {
  const states = {};
  for (const e of state.events) {
    const { source, record } = e.item;
    if (source === `${person.name}-host` && record.kind === "faults_armed") {
      for (const [name, on] of Object.entries(record.faults)) if (on) states[name] = "armed";
    } else if (source === `${person.name}-agent` && record.kind === "fault") {
      states[record.fault] = record.stage ? `fired when ${record.stage === "screening" ? "screening" : "authorizing"}`
        : record.outcome ? `fired: refused (${record.outcome})` : "fired: not refused";
    }
  }
  return states;
}

function faultsCard(person, states) {
  return h("div", {},
    h("h3", {}, "Fault switches"),
    h("p", { class: "hint" }, "Each switch waits for its kind of action, then fires once. Hover over one to see what it does."),
    h("div", { class: "faults" }, FAULTS.map(([name, label, help]) =>
      h("span", { class: "fault" },
        h("button", { class: "small", title: help, "aria-label": label, onclick: async () => {
          await api(`/v1/people/${person.name}/faults`, { method: "POST", body: JSON.stringify({ [name]: true }) });
        } }, label),
        states[name] ? pill(states[name], states[name] === "armed" ? "wait" : "deny") : null))));
}

// A section is rebuilt only when what it shows has changed, so a form being filled in, an open
// list and a button under the pointer survive the poll.
const drawn = new Map();

function changed(id, inputs) {
  const signature = JSON.stringify(inputs);
  if (drawn.get(id) === signature) return false;
  drawn.set(id, signature);
  return true;
}

const articles = new Map();

function renderPeople() {
  const container = document.getElementById("people");
  for (const person of state.overview.people) {
    let article = articles.get(person.name);
    if (!article) {
      article = { element: h("article", { class: "person" }), parts: new Map() };
      articles.set(person.name, article);
      container.append(article.element);
    }
    const status = person.status || {};
    const sources = [`${person.name}-agent`, `${person.name}-host`].map((s) => state.counts[s] || 0);
    const faults = faultStates(person);
    const parts = [
      ["head", [person.display, person.backend], () =>
        h("div", { class: "person-head" }, h("h2", {}, person.display), pill(person.backend))],
      ["request", [status.request, person.request_form], () => requestCard(person)],
      ["decisions", [status.negotiation, status.owner_decisions, status.peer, status.request], () => decisionsCard(person)],
      ["calendar", [person.events, person.calendar_error], () => calendarCard(person)],
      ["agent", [status.link, status.negotiation, sources], () => agentCard(person)],
      ["faults", faults, () => faultsCard(person, faults)],
    ];
    for (const [name, inputs, build] of parts) {
      if (!changed(`${person.name}/${name}`, inputs)) continue;
      const element = build();
      const old = article.parts.get(name);
      if (old) old.replaceWith(element);
      else article.element.append(element);
      article.parts.set(name, element);
    }
  }
}

function renderRelay() {
  if (!changed("relay", state.overview.relay)) return;
  const body = document.querySelector("#relay tbody");
  const frames = state.overview.relay.slice().reverse();
  body.replaceChildren(...(frames.length ? frames.map((f) => h("tr", {},
    h("td", { class: "mono" }, f.at), h("td", {}, f.to), h("td", { class: "mono" }, `${f.mailbox}…`),
    h("td", {}, f.seq), h("td", {}, f.bytes))) : [h("tr", {}, h("td", { colspan: 5, class: "empty" }, "No frames yet"))]));
}

function renderIsabelle() {
  const checks = state.events
    .filter((e) => e.item.source.endsWith("-host") && ["screen", "authorize"].includes(e.item.record.kind))
    .map((e) => e.item.record);
  if (!changed("isabelle", checks.length)) return;
  const seconds = checks.map((c) => c.seconds);
  const verdicts = checks.flatMap((c) => c.verdicts.map((v) => v.verdict.verdict));
  const stat = (value, name) => h("div", { class: "stat" }, h("div", { class: "value" }, value), h("div", { class: "name" }, name));
  const mean = seconds.length ? (seconds.reduce((a, b) => a + b, 0) / seconds.length).toFixed(2) : "—";
  const max = seconds.length ? Math.max(...seconds).toFixed(2) : "—";
  document.getElementById("isabelle").replaceChildren(
    stat(checks.length, "checks"),
    stat(`${mean} s`, `mean time (max ${max} s)`),
    stat(verdicts.filter((v) => v === "permit").length, "actions permitted"),
    stat(verdicts.filter((v) => v !== "permit").length, "actions denied or undecided"));
}

function summary(record) {
  switch (record.kind) {
    case "plans": return `turn ${record.turn}: ${record.plans.plans.length} plans, chosen ${record.chosen ?? "none"}`;
    case "authorize": return `${describe(record.action || (record.verdicts && record.verdicts[0] && record.verdicts[0].action))} → ${record.verdict ? record.verdict.verdict : record.verdicts[0].verdict.verdict}`;
    case "screen": return `${record.verdicts.length} actions in ${record.theory}`;
    case "executed": return describe(record.action);
    case "refused": return `${describe(record.action)}: ${record.detail}`;
    case "delivery": return `${record.delivery.event}`;
    case "send": case "handshake":
      if (record.outcome === "refused") return `${record.action} refused: ${record.error} (${record.detail})`;
      return record.kind === "send" ? `sent ${record.message.body.type}` : `session ${record.link}`;
    case "consent": return `owner said ${record.record.yes ? "yes" : "no"} to ${record.candidate}`;
    case "owner_request": return `request filed: ${record.request.duration_ms / 60000} minutes with ${record.request.peer}`;
    case "owner_action": return `${record.person}: ${record.action}${record.answer && record.answer.error ? ` refused (${record.answer.error})` : ""}`;
    case "agent_registered": return `agent binding key ${record.binding_key.slice(0, 12)}…`;
    case "faults_armed": return `fault switches armed: ${Object.keys(record.faults).filter((f) => record.faults[f]).join(", ")}`;
    case "fault": return `${record.fault} fired${record.stage ? ` (${record.stage})` : `: ${record.outcome ? `refused with ${record.outcome}` : "not refused"}`}`;
    case "plan_failed": return `planner failed: ${record.detail}`;
    case "gave_up": return `gave up after ${record.attempts} attempts`;
    case "error": case "relay_error": return record.detail;
    case "replayed_call_ignored": return `replayed call ignored (session ${record.session.slice(0, 8)}…)`;
    default: break;
  }
  if (record.mailbox) return `frame ${record.seq} to ${record.mailbox.slice(0, 8)}…, ${record.bytes} bytes`;
  if (record.outcome) return `${record.endpoint || ""} ${record.action || ""} ${record.outcome} ${record.error || ""}`;
  return JSON.stringify(record).slice(0, 160);
}

// Relay records are frames; calendar records are requests to an endpoint.
function kindOf(record) {
  return record.kind || (record.mailbox ? "frame" : record.endpoint ? "request" : "record");
}

function renderAudit() {
  const source = document.getElementById("filter-source").value;
  const kind = document.getElementById("filter-kind").value;
  if (!changed("audit", [state.lastSeq, source, kind])) return;
  const sources = new Set(), kinds = new Set();
  for (const e of state.events) { sources.add(e.item.source); kinds.add(kindOf(e.item.record)); }
  for (const [id, values] of [["filter-source", sources], ["filter-kind", kinds]]) {
    const sorted = [...values].sort();
    if (!changed(id, sorted)) continue;
    const select = document.getElementById(id);
    const current = select.value;
    select.replaceChildren(h("option", { value: "" }, "all"), ...sorted.map((v) => h("option", { value: v }, v)));
    select.value = current;
  }
  const rows = state.events
    .filter((e) => (!source || e.item.source === source) && (!kind || kindOf(e.item.record) === kind))
    .slice(-300).reverse()
    .map((e) => h("tr", {},
      h("td", {}, e.seq), h("td", {}, e.item.source), h("td", {}, kindOf(e.item.record)),
      h("td", { class: "summary" }, summary(e.item.record))));
  document.querySelector("#audit tbody").replaceChildren(...rows);
}

function render() {
  if (!state.overview) return;
  document.getElementById("clock").textContent = state.overview.now;
  document.getElementById("start").disabled = state.overview.people.every((p) => p.status && p.status.request);
  document.getElementById("backends").replaceChildren(...state.overview.people.map((p) => pill(`${p.display}: ${p.backend}`)));
  renderPeople();
  renderRelay();
  renderIsabelle();
  renderAudit();
}

function connection(text, kind) {
  const element = document.getElementById("connection");
  element.textContent = text;
  element.className = `pill ${kind}`;
}

let stream = null;

async function refresh() {
  try {
    const overview = await api("/v1/overview");
    if (overview.error) throw new Error(overview.detail || overview.error);
    state.overview = overview;
    if (stream && stream.readyState === EventSource.OPEN) connection("live", "permit");
    render();
  } catch (error) {
    connection(`error: ${error.message}`, "deny");
  }
}

function connect() {
  const source = new EventSource(`/v1/stream?key=${encodeURIComponent(key)}&after=${state.lastSeq}`);
  stream = source;
  source.onopen = () => connection("live", "permit");
  source.onmessage = (message) => {
    const event = JSON.parse(message.data);
    if (event.seq <= state.lastSeq) return;
    state.lastSeq = event.seq;
    state.events.push(event);
    state.counts[event.item.source] = (state.counts[event.item.source] || 0) + 1;
    if (state.events.length > 5000) state.events.shift();
    scheduleRender();
  };
  source.onerror = () => connection("reconnecting…", "wait");
}

let pending = false;
function scheduleRender() {
  if (pending) return;
  pending = true;
  setTimeout(() => { pending = false; render(); }, 250);
}

// Files every request not filed yet, with what each form shows.
document.getElementById("start").addEventListener("click", async () => {
  for (const person of state.overview ? state.overview.people : []) {
    const form = articles.get(person.name)?.parts.get("request")?.querySelector("form");
    if (form && !(person.status && person.status.request)) await fileRequest(person.name, form);
  }
  refresh();
});

document.getElementById("reset").addEventListener("click", async () => {
  for (const person of state.overview ? state.overview.people : []) {
    await api(`/v1/people/${person.name}/reset`, { method: "POST", body: "{}" });
  }
  refresh();
});

for (const id of ["filter-source", "filter-kind"]) {
  document.getElementById(id).addEventListener("change", renderAudit);
}

if (!key) {
  connection("no key: open the URL that fabric-demo up printed", "deny");
} else {
  refresh();
  setInterval(refresh, 1500);
  connect();
}
