type EventHandler = (data: unknown) => void;

interface WsMessage {
  id?: string;
  type: string;
  data?: unknown;
  error?: { message: string };
}

let ws: WebSocket | null = null;
let reqId = 0;
let pingTimer: ReturnType<typeof setInterval> | null = null;
let wasConnected = false;
let intentionalClose = false;

const PING_INTERVAL = 15_000;
const REQUEST_TIMEOUT = 60_000;
const RECONNECT_GRACE_MS = 3_000;
const RECONNECT_MIN_MS = 500;
const RECONNECT_MAX_MS = 15_000;

let reconnectUrl: string | null = null;
let reconnectToken: string | undefined;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectDelay = RECONNECT_MIN_MS;
let reconnecting = false;
let graceTimer: ReturnType<typeof setTimeout> | null = null;

const pending = new Map<
  string,
  { resolve: (v: unknown) => void; reject: (e: Error) => void }
>();
const listeners = new Map<string, Set<EventHandler>>();

function jitteredDelay(baseMs: number): number {
  // Add +-20% jitter to avoid reconnect stampedes.
  const jitterFactor = 0.8 + Math.random() * 0.4;
  return Math.max(RECONNECT_MIN_MS, Math.floor(baseMs * jitterFactor));
}

function startPing() {
  stopPing();
  pingTimer = setInterval(() => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      const id = String(++reqId);
      ws.send(JSON.stringify({ id, method: "ping" }));
    }
  }, PING_INTERVAL);
}

function stopPing() {
  if (pingTimer !== null) {
    clearInterval(pingTimer);
    pingTimer = null;
  }
}

function cancelReconnect() {
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
  if (graceTimer) { clearTimeout(graceTimer); graceTimer = null; }
  reconnecting = false;
  reconnectDelay = RECONNECT_MIN_MS;
}

function scheduleReconnect() {
  if (reconnecting || !reconnectUrl || intentionalClose) return;
  reconnecting = true;

  graceTimer = setTimeout(() => {
    graceTimer = null;
    if (reconnecting) emit("disconnected", null);
  }, RECONNECT_GRACE_MS);

  const attempt = () => {
    if (intentionalClose || !reconnectUrl) { cancelReconnect(); return; }
    if (ws && ws.readyState === WebSocket.OPEN) { cancelReconnect(); return; }

    doConnect(reconnectUrl!, reconnectToken)
      .then(() => {
        if (graceTimer) { clearTimeout(graceTimer); graceTimer = null; }
        cancelReconnect();
        emit("reconnected", null);
      })
      .catch(() => {
        reconnectDelay = Math.min(reconnectDelay * 1.5, RECONNECT_MAX_MS);
        reconnectTimer = setTimeout(attempt, jitteredDelay(reconnectDelay));
      });
  };

  reconnectTimer = setTimeout(attempt, jitteredDelay(RECONNECT_MIN_MS));
}

function doConnect(url: string, token?: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const fullUrl = token ? `${url}?token=${encodeURIComponent(token)}` : url;
    const socket = new WebSocket(fullUrl);
    let resolved = false;

    socket.onopen = () => {
      ws = socket;
    };

    socket.onmessage = (ev) => {
      let msg: WsMessage;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }

      if (msg.type === "connected") {
        wasConnected = true;
        startPing();
        emit("connected", msg.data);
        resolved = true;
        resolve();
        return;
      }

      if (msg.type === "heartbeat" || msg.type === "pong") return;

      if (msg.id && pending.has(msg.id)) {
        const p = pending.get(msg.id)!;
        pending.delete(msg.id);
        if (msg.type.endsWith(".error") || msg.type === "error") {
          p.reject(new Error(msg.error?.message ?? "unknown error"));
        } else {
          p.resolve(msg);
        }
      }

      // Broadcast events carry `type:"event"` plus an `event` field for routing.
      // Re-emit by the event name so listeners registered by event name fire correctly.
      if (msg.type === "event" && typeof (msg as Record<string, unknown>)["event"] === "string") {
        emit((msg as Record<string, unknown>)["event"] as string, msg);
      } else {
        emit(msg.type, msg);
      }
    };

    socket.onclose = () => {
      const wasPrev = wasConnected;
      ws = null;
      wasConnected = false;
      stopPing();
      if (pending.size > 0) {
        for (const [, req] of pending) {
          req.reject(new Error("WebSocket closed"));
        }
        pending.clear();
      }
      if (!resolved) {
        resolved = true;
        reject(new Error("WebSocket connection failed"));
      }
      if (wasPrev && !intentionalClose) {
        scheduleReconnect();
      }
    };

    socket.onerror = () => {
      console.warn("[ws-client] socket error");
    };
  });
}

export function connect(url: string, token?: string): Promise<void> {
  intentionalClose = true;
  cancelReconnect();
  if (ws) {
    try { ws.close(); } catch { /* ignore */ }
    ws = null;
  }
  stopPing();
  intentionalClose = false;

  reconnectUrl = url;
  reconnectToken = token;

  return doConnect(url, token);
}

export function disconnect() {
  intentionalClose = true;
  cancelReconnect();
  stopPing();
  ws?.close();
  ws = null;
  reconnectUrl = null;
}

export function send(
  method: string,
  params?: Record<string, unknown>,
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error("WebSocket not connected"));
      return;
    }
    const id = String(++reqId);
    pending.set(id, { resolve, reject });
    try {
      ws.send(JSON.stringify({ id, method, params }));
    } catch (err) {
      pending.delete(id);
      reject(err instanceof Error ? err : new Error("failed to send ws message"));
      return;
    }
    setTimeout(() => {
      if (pending.has(id)) {
        pending.delete(id);
        reject(new Error("timeout"));
      }
    }, REQUEST_TIMEOUT);
  });
}

export function on(event: string, handler: EventHandler) {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(handler);
  return () => listeners.get(event)?.delete(handler);
}

function emit(event: string, data: unknown) {
  listeners.get(event)?.forEach((h) => h(data));
}

export function isConnected(): boolean {
  return ws !== null && ws.readyState === WebSocket.OPEN;
}
