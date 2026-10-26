export function mount(container, api) {
  const view = document.createElement("section");
  view.innerHTML = '<p>Fixture frontend</p><output></output><div></div>';
  const output = view.querySelector("output");
  const actions = view.querySelector("div");
  const pending = [];
  let cancel;
  let resume;
  let closed = false;
  function button(label, action) {
    const button = document.createElement("button");
    button.textContent = label;
    button.onclick = () => { void action().catch(() => { output.textContent = "failed"; }); };
    actions.append(button);
  }
  for (const [label, action] of [
    ["Text", () => api.requestText({ title: "Text", prompt: "Test text" })],
    ["Selection", () => api.requestSelection({ title: "Selection", prompt: "Choose", options: [{value:"a",label:"Alpha"},{value:"b",label:"Beta"}] })],
    ["Confirmation", () => api.requestConfirmation({ title: "Confirmation", prompt: "Confirm?" })],
  ]) button(label, async () => { const result = await action(); if (!closed) output.textContent = JSON.stringify(result); });
  button("Hold events", async () => {
    const gate = new Promise((resolve) => { resume = resolve; });
    cancel = await api.subscribeEvents(async (event) => {
      pending.push(event.name);
      view.dataset.events = JSON.stringify(pending);
      await gate;
    });
    view.dataset.subscribed = "true";
  });
  button("Resume events", async () => { resume?.(); });
  button("Flood", async () => { await api.invoke("fixture.flood", {}); });
  container.append(view);
  return { unmount() { closed = true; resume?.(); cancel?.(); view.remove(); } };
}
