/**
 * Advanced middleware/decorator pattern with dead middleware functions,
 * closures, function-as-argument patterns, and self-referencing dead chains.
 */

// --- Live middleware ---

function authMiddleware(req, res, next) {
  const token = req.headers["authorization"];
  if (!token) {
    res.status(401).json({ error: "Unauthorized" });
    return;
  }
  const decoded = verifyToken(token);
  if (!decoded) {
    res.status(401).json({ error: "Invalid token" });
    return;
  }
  req.user = decoded;
  next();
}

function corsMiddleware(req, res, next) {
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type, Authorization");
  if (req.method === "OPTIONS") {
    res.status(204).end();
    return;
  }
  next();
}

// --- Dead middleware: defined but never registered ---

function rateLimitMiddleware(req, res, next) {
  const clientIp = req.headers["x-forwarded-for"] || req.connection.remoteAddress;
  // Dead store: extracted but never used
  const userAgent = req.headers["user-agent"];
  const path = req.url;

  const key = `${clientIp}:${path}`;
  const count = getRateCount(key);
  if (count > 100) {
    res.status(429).json({ error: "Rate limit exceeded" });
    return;
  }
  incrementRateCount(key);
  next();
}

function cacheMiddleware(req, res, next) {
  const cacheKey = req.method + ":" + req.url;
  // Dead store: extracted but never used
  const acceptHeader = req.headers["accept"];
  const ifNoneMatch = req.headers["if-none-match"];

  const cached = getFromCache(cacheKey);
  if (cached && !ifNoneMatch) {
    res.json(cached);
    return;
  }
  next();
}

// --- Clone of authMiddleware with different inner check ---

function roleMiddleware(req, res, next) {
  const token = req.headers["authorization"];
  if (!token) {
    res.status(401).json({ error: "Unauthorized" });
    return;
  }
  const decoded = verifyToken(token);
  if (!decoded) {
    res.status(401).json({ error: "Invalid token" });
    return;
  }
  req.role = decoded.role;
  next();
}

// --- Dead self-referencing chain ---

function retryWithBackoff(fn, attempt, maxAttempts) {
  if (attempt >= maxAttempts) {
    return { success: false, error: "Max retries exceeded" };
  }
  try {
    return fn();
  } catch (err) {
    const delay = Math.pow(2, attempt) * 100;
    setTimeout(() => {
      retryWithBackoff(fn, attempt + 1, maxAttempts);
    }, delay);
  }
}

// --- Dead closure pattern ---

function createRateLimiter(maxRequests, windowMs) {
  const requests = new Map();

  return function limiter(clientId) {
    const now = Date.now();
    const windowStart = now - windowMs;
    const clientRequests = (requests.get(clientId) || []).filter(
      (t) => t > windowStart,
    );
    clientRequests.push(now);
    requests.set(clientId, clientRequests);
    return clientRequests.length <= maxRequests;
  };
}

// --- Stub helpers (simulating external deps) ---

function verifyToken(token) {
  if (token && token.startsWith("Bearer ")) {
    return { id: "user-1", role: "admin" };
  }
  return null;
}

function getRateCount(key) {
  return 0;
}

function incrementRateCount(key) {
  // no-op stub
}

function getFromCache(key) {
  return null;
}

// --- Route handlers ---

function handleGetUsers(req, res) {
  res.json({ users: [{ id: 1, name: "Alice" }] });
}

function handleCreateUser(req, res) {
  const { name, email } = req.body;
  res.status(201).json({ id: 2, name, email });
}

function handleDeleteUser(req, res) {
  const { id } = req.params;
  res.status(204).end();
}

// --- App setup (entry point) ---

function createApp() {
  const app = {
    middlewares: [],
    routes: {},
    use(mw) {
      this.middlewares.push(mw);
    },
    get(path, handler) {
      this.routes["GET:" + path] = handler;
    },
    post(path, handler) {
      this.routes["POST:" + path] = handler;
    },
    delete(path, handler) {
      this.routes["DELETE:" + path] = handler;
    },
  };

  // Only authMiddleware and corsMiddleware are registered
  // rateLimitMiddleware, cacheMiddleware, roleMiddleware are NOT registered
  app.use(authMiddleware);
  app.use(corsMiddleware);

  app.get("/users", handleGetUsers);
  app.post("/users", handleCreateUser);
  app.delete("/users/:id", handleDeleteUser);

  return app;
}

const app = createApp();
console.log(
  "App created with",
  app.middlewares.length,
  "middleware and",
  Object.keys(app.routes).length,
  "routes",
);
