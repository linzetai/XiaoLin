import { test as base, type Page, type Route } from "@playwright/test";
import { MOCK_AGENTS, MOCK_SESSIONS, MOCK_MODELS, MOCK_MESSAGES } from "./mock-data";

/**
 * Build the RPC response table that the mock WebSocket uses.
 * The real gateway uses a WS RPC protocol:
 *   client → { id, method, params }
 *   server → { id, type: method, data: {...} }
 *
 * Additionally, after opening the connection, the server must send
 *   { type: "connected" }
 * to signal readiness.
 */
function buildRpcHandlers(): string {
  const agents = JSON.stringify(MOCK_AGENTS);
  const sessions = JSON.stringify(MOCK_SESSIONS);
  const models = JSON.stringify(MOCK_MODELS);
  const messages = JSON.stringify(MOCK_MESSAGES);

  return `
    const __MOCK_AGENTS__ = ${agents};
    const __MOCK_SESSIONS__ = ${sessions};
    const __MOCK_MODELS__ = ${models};
    const __MOCK_MESSAGES__ = ${messages};

    function handleRpc(msg) {
      const id = msg.id;
      const method = msg.method;
      const params = msg.params || {};

      switch (method) {
        case "ping":
          return { id, type: "pong" };

        case "agents":
          return { id, type: "agents", data: { agents: __MOCK_AGENTS__ } };

        case "sessions.list":
          return { id, type: "sessions.list", data: { sessions: __MOCK_SESSIONS__ } };

        case "sessions.get":
          return { id, type: "sessions.get", data: __MOCK_SESSIONS__.find(s => s.id === params.sessionId) || null };

        case "sessions.messages":
          return { id, type: "sessions.messages", data: { messages: __MOCK_MESSAGES__ } };

        case "config.get":
          if (params.key === "onboarding") {
            return { id, type: "config.get", data: { key: "onboarding", value: { completed: true } } };
          }
          return { id, type: "config.get", data: { key: params.key, value: null } };

        case "config.set":
          return { id, type: "config.set", data: { persisted: true, pendingRestart: false } };

        case "models.list":
          return { id, type: "models.list", data: { models: __MOCK_MODELS__ } };

        default:
          return { id, type: method, data: {} };
      }
    }
  `;
}

async function injectMockTransport(page: Page) {
  const rpcHandlers = buildRpcHandlers();

  await page.addInitScript(`
    (function() {
      ${rpcHandlers}

      var OrigWS = window.WebSocket;

      function MockWebSocket(url, protocols) {
        this.url = typeof url === "string" ? url : url.toString();
        this.readyState = 0; // CONNECTING
        this.protocol = "";
        this.binaryType = "blob";
        this.bufferedAmount = 0;
        this.extensions = "";
        this.onopen = null;
        this.onclose = null;
        this.onmessage = null;
        this.onerror = null;
        this._listeners = {};

        var self = this;

        // Simulate async open + "connected" handshake
        Promise.resolve().then(function() {
          self.readyState = 1; // OPEN
          var openEvt = new Event("open");
          if (self.onopen) self.onopen(openEvt);
          self._dispatch("open", openEvt);

          // Send the "connected" message (required by ws-client doConnect)
          Promise.resolve().then(function() {
            var connMsg = JSON.stringify({ type: "connected", data: {} });
            var connEvt = new MessageEvent("message", { data: connMsg });
            if (self.onmessage) self.onmessage(connEvt);
            self._dispatch("message", connEvt);
          });
        });
      }

      MockWebSocket.CONNECTING = 0;
      MockWebSocket.OPEN = 1;
      MockWebSocket.CLOSING = 2;
      MockWebSocket.CLOSED = 3;

      MockWebSocket.prototype.CONNECTING = 0;
      MockWebSocket.prototype.OPEN = 1;
      MockWebSocket.prototype.CLOSING = 2;
      MockWebSocket.prototype.CLOSED = 3;

      MockWebSocket.prototype.addEventListener = function(type, listener) {
        if (!this._listeners[type]) this._listeners[type] = new Set();
        this._listeners[type].add(listener);
      };

      MockWebSocket.prototype.removeEventListener = function(type, listener) {
        if (this._listeners[type]) this._listeners[type].delete(listener);
      };

      MockWebSocket.prototype.send = function(data) {
        if (this.readyState !== 1) return;
        var self = this;
        try {
          var msg = JSON.parse(data);
          var response = handleRpc(msg);
          if (response) {
            Promise.resolve().then(function() {
              var reply = JSON.stringify(response);
              var ev = new MessageEvent("message", { data: reply });
              if (self.onmessage) self.onmessage(ev);
              self._dispatch("message", ev);
            });
          }
        } catch(e) {
          // non-JSON — ignore
        }
      };

      MockWebSocket.prototype.close = function(code, reason) {
        if (this.readyState >= 2) return;
        this.readyState = 2; // CLOSING
        var self = this;
        Promise.resolve().then(function() {
          self.readyState = 3; // CLOSED
          var ev = new CloseEvent("close", { code: code || 1000, wasClean: true });
          if (self.onclose) self.onclose(ev);
          self._dispatch("close", ev);
        });
      };

      MockWebSocket.prototype.dispatchEvent = function(event) {
        this._dispatch(event.type, event);
        return true;
      };

      MockWebSocket.prototype._dispatch = function(type, event) {
        var set = this._listeners[type];
        if (set) {
          set.forEach(function(fn) {
            if (typeof fn === "function") fn(event);
            else fn.handleEvent(event);
          });
        }
      };

      Object.defineProperty(window, "WebSocket", {
        value: MockWebSocket,
        writable: true,
        configurable: true,
      });

      window.__ORIG_WS__ = OrigWS;
      window.__MOCK_WS__ = MockWebSocket;
    })();
  `);
}

async function setupApiRoutes(page: Page) {
  await page.route("http://127.0.0.1:18888/health", (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ status: "ok" }),
    }),
  );

  await page.route("**/api/**", (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "{}",
    }),
  );
}

export const test = base.extend<{ mockGateway: void }>({
  mockGateway: [
    async ({ page }, use) => {
      await injectMockTransport(page);
      await setupApiRoutes(page);
      await use();
    },
    { auto: true },
  ],
});

export { expect } from "@playwright/test";

export async function waitForAppReady(page: Page) {
  await page.goto("/");
  await page.waitForSelector("main", {
    state: "visible",
    timeout: 15_000,
  });
  await page.waitForTimeout(500);
}
