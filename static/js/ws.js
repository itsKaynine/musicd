class ReconnectingWebSocket {
  constructor(url, protocols = []) {
    this.url = url;
    this.protocols = protocols;
    this.ws = null;

    this.reconnectDelay = 1000;
    this.maxDelay = 30000;
    this.heartbeatInterval = 25000;
    this.shouldReconnect = true;

    this.listeners = {
      open: [],
      close: [],
      message: [],
      error: []
    };

    this.connect();
  }

  connect() {
    this.ws = new WebSocket(this.url, this.protocols);

    this.ws.onopen = (event) => {
      this.reconnectDelay = 1000; // reset backoff
      this.startHeartbeat();
      this._emit("open", event);
    };

    this.ws.onmessage = (event) => {
      this._emit("message", event);
    };

    this.ws.onerror = (event) => {
      this._emit("error", event);
    };

    this.ws.onclose = (event) => {
      this.stopHeartbeat();
      this._emit("close", event);

      if (this.shouldReconnect) {
        setTimeout(() => this.connect(), this.reconnectDelay);
        this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxDelay);
      }
    };
  }

  send(data) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(data);
    } else {
      console.warn("WebSocket not open, cannot send:", data);
    }
  }

  close(code = 1000, reason) {
    this.shouldReconnect = false;
    this.stopHeartbeat();
    if (this.ws) {
      this.ws.close(code, reason);
    }
  }

  startHeartbeat() {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      if (this.ws.readyState === WebSocket.OPEN) {
        this.ws.send(JSON.stringify({ type: "ping" }));
      }
    }, this.heartbeatInterval);
  }

  stopHeartbeat() {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  on(event, handler) {
    if (this.listeners[event]) {
      this.listeners[event].push(handler);
    }
  }

  off(event, handler) {
    if (this.listeners[event]) {
      this.listeners[event] = this.listeners[event].filter(h => h !== handler);
    }
  }

  _emit(event, arg) {
    if (this.listeners[event]) {
      this.listeners[event].forEach(h => h(arg));
    }
  }
}
