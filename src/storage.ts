import type { SavedConnection } from "./types";

const KEY = "xvncviewer:connections";

export function readConnections(): SavedConnection[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];

    return parsed.map((item) => ({
      ...item,
      host: typeof item.host === "string" ? item.host : "127.0.0.1",
      port: typeof item.port === "number" ? item.port : 5900,
      wsUrl:
        typeof item.wsUrl === "string"
          ? item.wsUrl
          : "ws://127.0.0.1:6081/websockify",
      username: typeof item.username === "string" ? item.username : "",
    }));
  } catch {
    return [];
  }
}

export function writeConnections(connections: SavedConnection[]) {
  localStorage.setItem(KEY, JSON.stringify(connections));
}

export function upsertConnection(connection: SavedConnection) {
  const next = [
    connection,
    ...readConnections().filter((item) => item.id !== connection.id),
  ].slice(0, 24);
  writeConnections(next);
  return next;
}
