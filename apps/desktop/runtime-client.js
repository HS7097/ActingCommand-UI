// SPDX-License-Identifier: AGPL-3.0-only
window.GachaPilotRuntimeClient = (() => {
  const params = new URLSearchParams(window.location.search);
  const httpBase = (params.get("runtime") || localStorage.getItem("gachapilot.runtime.http") || "http://127.0.0.1:8765").replace(/\/$/, "");
  const wsBase = (params.get("events") || localStorage.getItem("gachapilot.runtime.ws") || "ws://127.0.0.1:8766/events").replace(/\/$/, "");

  async function request(path, options = {}) {
    const response = await fetch(`${httpBase}${path}`, {
      ...options,
      headers: {
        "Content-Type": "application/json",
        ...(options.headers || {}),
      },
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(body.error || `Runtime API failed: ${response.status}`);
    }
    return body;
  }

  async function loadSnapshot() {
    try {
      const [health, profileResponse, status, logResponse, historyResponse, acquisitionResponse] = await Promise.all([
        request("/health"),
        request("/profiles"),
        request("/runtime/status"),
        request("/logs/recent?limit=80"),
        request("/resources/history"),
        request("/acquisitions/recent?limit=24"),
      ]);
      return {
        degraded: false,
        degradedReason: "",
        health,
        status,
        profiles: profileResponse.profiles || [],
        logs: logResponse.logs || [],
        resourceHistory: historyResponse.history || [],
        acquisitions: acquisitionResponse.acquisitions || [],
      };
    } catch (error) {
      console.warn("AliceRuntimeOrchestrator unavailable", error);
      return window.GachaPilotMockData.snapshot(error.message);
    }
  }

  async function command(name) {
    const commandMap = {
      start: "/runtime/start",
      stop: "/runtime/stop",
      restart: "/runtime/restart",
      refresh: "/runtime/refresh",
    };
    const path = commandMap[name];
    if (!path) {
      throw new Error(`Unknown runtime command: ${name}`);
    }
    return request(path, { method: "POST", body: "{}" });
  }

  function connectEvents(onEvent, onError) {
    let socket;
    try {
      socket = new WebSocket(wsBase);
    } catch (error) {
      onError?.(error);
      return () => {};
    }
    socket.addEventListener("message", (event) => {
      try {
        onEvent(JSON.parse(event.data));
      } catch (error) {
        onError?.(error);
      }
    });
    socket.addEventListener("error", () => {
      onError?.(new Error("WebSocket event stream failed"));
    });
    socket.addEventListener("close", () => {
      onError?.(new Error("WebSocket event stream closed"));
    });
    return () => socket.close();
  }

  return {
    httpBase,
    wsBase,
    loadSnapshot,
    command,
    connectEvents,
  };
})();
