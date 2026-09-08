/* Nodel master-volume slider. Export the same state model for node --test. */
(function (root) {
  "use strict";

  function Panel(client) {
    this.client = client;
    this.sequence = 0;
    this.state = null;
    this.pending = null;
    this.queue = [];
    this.dragging = false;
  }
  Panel.prototype.input = function (db, final) {
    if (!this.state || !this.state.available) return;
    var action = { db: Number(db), final: final, client: this.client,
      sequence: ++this.sequence, session: this.state.session };
    this.pending = action;
    this.dragging = !final;
    // Coalesce unsent previews, but never discard a queued final adjustment.
    if (this.queue.length && !this.queue[this.queue.length - 1].final) this.queue.pop();
    this.queue.push(action);
  };
  Panel.prototype.update = function (state) {
    if (this.state && state.session === this.state.session && state.revision <= this.state.revision) return;
    if (this.state && state.session !== this.state.session) {
      this.pending = null;
      this.queue = [];
      this.dragging = false;
    }
    this.state = state;
    if (!state.available) {
      this.pending = null;
      this.queue = [];
      this.dragging = false;
    } else if (this.pending && !this.dragging && state.client === this.client &&
        state.sequence >= this.pending.sequence && state.final && state.pendingDb == null) {
      this.pending = null;
    }
  };
  Panel.prototype.disconnected = function () {
    this.pending = null;
    this.queue = [];
    this.dragging = false;
    if (this.state) this.state.available = false;
  };
  Panel.prototype.value = function () {
    return this.pending ? this.pending.db : this.state && this.state.confirmedDb;
  };

  function mount() {
    var slider = document.querySelector('input[type="range"][data-action="Volume"]');
    if (!slider) return;
    var panel = new Panel(Date.now().toString(36) + Math.random().toString(36).slice(2));
    var readout = document.createElement("p");
    readout.setAttribute("role", "status");
    slider.parentNode.parentNode.appendChild(readout);
    slider.setAttribute("aria-label", "CuePool master volume in decibels");
    slider.disabled = true;
    var sending = false;
    var nextPreviewAt = 0;
    var pendingSince = 0;
    var stopped = false;

    function draw() {
      var value = panel.value();
      if (value !== null && value !== undefined) slider.value = value;
      slider.disabled = !panel.state || !panel.state.available || typeof value !== "number";
      var label = value <= -96 ? "Muted" : Number(value).toFixed(1) + " dB";
      readout.textContent = slider.disabled ? "Volume feedback unavailable" :
        label + (panel.pending ? " — awaiting confirmation" : "");
    }
    function request(method, path, body, done) {
      var xhr = new XMLHttpRequest();
      xhr.open(method, path);
      xhr.timeout = 2000;
      xhr.setRequestHeader("Cache-Control", "no-cache");
      if (body !== null) xhr.setRequestHeader("Content-Type", "application/json");
      xhr.onload = function () {
        if (xhr.status < 200 || xhr.status >= 300) return done(new Error("HTTP " + xhr.status));
        try { done(null, xhr.responseText ? JSON.parse(xhr.responseText) : null); }
        catch (error) { done(error); }
      };
      xhr.onerror = xhr.ontimeout = function () { done(new Error("Nodel unavailable")); };
      xhr.send(body === null ? null : JSON.stringify(body));
    }
    function sendNext() {
      if (stopped || sending || !panel.queue.length) return;
      if (!panel.queue[0].final && Date.now() < nextPreviewAt) return;
      var action = panel.queue.shift();
      sending = true;
      nextPreviewAt = Date.now() + 100;
      request("POST", "REST/actions/Volume/call", {arg: action}, function (error) {
        sending = false;
        if (error) panel.disconnected();
        draw();
        sendNext();
      });
    }
    function input(event) {
      // Stop the stock Nodel body handler from sending a second action. This
      // slider's action schema includes correlation fields, not just a number.
      event.stopImmediatePropagation();
      var final = event.type !== "input";
      panel.input(slider.value, final);
      pendingSince = Date.now();
      draw();
      sendNext();
    }
    slider.addEventListener("input", input, true);
    slider.addEventListener("change", input, true);
    // A cancelled touch/drag also commits its last visible value.
    slider.addEventListener("pointercancel", input, true);
    slider.addEventListener("blur", function (event) {
      if (panel.dragging) input(event);
    }, true);

    function poll() {
      if (stopped) return;
      request("GET", "REST/events", null, function (error, events) {
        if (error || !events || !events.VolumeState) panel.disconnected();
        else panel.update(events.VolumeState.arg);
        // Also recover a panel whose action never reached Nodel or which lost
        // arbitration to another panel. Do not retain unconfirmed input forever.
        if (panel.pending && !panel.dragging && Date.now() - pendingSince > 5000) {
          panel.pending = null;
          panel.queue = [];
        }
        draw();
        if (!stopped) setTimeout(poll, 250);
      });
    }
    var sender = setInterval(sendNext, 50);
    root.addEventListener("pagehide", function () { stopped = true; clearInterval(sender); });
    draw();
    poll();
  }
  if (typeof module !== "undefined" && module.exports) module.exports = Panel;
  else if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount);
  else mount();
}(typeof window !== "undefined" ? window : globalThis));
