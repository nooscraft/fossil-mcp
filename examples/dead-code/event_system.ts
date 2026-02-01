/**
 * Advanced event system with dead subclass, dead interface impl,
 * subtle structural clones, and dead destructured stores.
 */

// --- Interfaces ---

interface Event {
  type: string;
  timestamp: number;
  payload: Record<string, unknown>;
}

interface EventHandler {
  handle(event: Event): void;
}

interface ICollector {
  collect(metric: string, value: number): void;
  flush(): void;
}

// --- Live: EventBus ---

class EventBus {
  private handlers: Map<string, EventHandler[]> = new Map();

  on(eventType: string, handler: EventHandler): void {
    const existing = this.handlers.get(eventType) || [];
    existing.push(handler);
    this.handlers.set(eventType, existing);
  }

  emit(event: Event): void {
    const handlers = this.handlers.get(event.type) || [];
    for (const handler of handlers) {
      handler.handle(event);
    }
  }
}

// --- Live handlers with subtle clones ---

class UserEventHandler implements EventHandler {
  handle(event: Event): void {
    const { type, timestamp, payload } = event;
    const userId = payload.userId as string;
    const action = payload.action as string;

    if (!userId || !action) {
      console.error(`Invalid user event at ${timestamp}`);
      return;
    }

    console.log(`[${type}] User ${userId} performed ${action}`);

    if (action === "login") {
      this.trackLogin(userId, timestamp);
    }
  }

  private trackLogin(userId: string, timestamp: number): void {
    console.log(`Login tracked: ${userId} at ${timestamp}`);
  }
}

class OrderEventHandler implements EventHandler {
  handle(event: Event): void {
    const { type, timestamp, payload } = event;
    const orderId = payload.orderId as string;
    const status = payload.status as string;

    if (!orderId || !status) {
      console.error(`Invalid order event at ${timestamp}`);
      return;
    }

    console.log(`[${type}] Order ${orderId} changed to ${status}`);

    if (status === "completed") {
      this.trackCompletion(orderId, timestamp);
    }
  }

  private trackCompletion(orderId: string, timestamp: number): void {
    console.log(`Completion tracked: ${orderId} at ${timestamp}`);
  }
}

// --- Dead subclass: extends EventHandler but never registered ---

class AuditEventHandler implements EventHandler {
  private auditLog: string[] = [];

  handle(event: Event): void {
    const { type, timestamp, payload } = event;
    const actor = payload.actor as string;
    const resource = payload.resource as string;

    if (!actor || !resource) {
      console.error(`Invalid audit event at ${timestamp}`);
      return;
    }

    console.log(`[${type}] Actor ${actor} accessed ${resource}`);

    if (resource === "admin") {
      this.flagSensitiveAccess(actor, timestamp);
    }
  }

  private flagSensitiveAccess(actor: string, timestamp: number): void {
    this.auditLog.push(`SENSITIVE: ${actor} at ${timestamp}`);
  }

  getAuditLog(): string[] {
    return [...this.auditLog];
  }
}

// --- Dead interface implementation: never instantiated ---

class MetricsCollector implements ICollector {
  private buffer: Array<{ metric: string; value: number }> = [];

  collect(metric: string, value: number): void {
    this.buffer.push({ metric, value });
  }

  flush(): void {
    for (const entry of this.buffer) {
      console.log(`Metric: ${entry.metric} = ${entry.value}`);
    }
    this.buffer = [];
  }
}

// --- Dead stores with destructuring ---

function processEventBatch(events: Event[]): number {
  let processed = 0;
  for (const event of events) {
    // Dead destructured stores: timestamp and payload are never read
    const { type, timestamp, payload } = event;
    if (type === "user" || type === "order") {
      processed++;
    }
  }
  return processed;
}

// --- Dead utility ---

function formatEventForExport(event: Event): string {
  const { type, timestamp, payload } = event;
  const fields = Object.keys(payload).join(", ");
  return `${type}|${timestamp}|${fields}`;
}

// --- Entry point ---

function main(): void {
  const bus = new EventBus();

  // Only UserEventHandler and OrderEventHandler are registered
  // AuditEventHandler is never registered — dead
  bus.on("user", new UserEventHandler());
  bus.on("order", new OrderEventHandler());

  bus.emit({
    type: "user",
    timestamp: Date.now(),
    payload: { userId: "u123", action: "login" },
  });

  bus.emit({
    type: "order",
    timestamp: Date.now(),
    payload: { orderId: "ord-456", status: "completed" },
  });
}

main();
