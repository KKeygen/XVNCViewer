import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Clipboard,
  Command,
  Fullscreen,
  History,
  Keyboard,
  Monitor,
  MonitorUp,
  Play,
  Plus,
  Square,
  X,
} from "lucide-react";
import { readConnections, upsertConnection, writeConnections } from "./storage";
import type { ConnectionState, SavedConnection } from "./types";
import { useVncSession } from "./useVncSession";

const defaultConnections: SavedConnection[] = [
  {
    id: "local-macos",
    name: "Local macOS Screen Sharing",
    host: "127.0.0.1",
    port: 5900,
    wsUrl: "",
    username: "",
  },
];

interface ProxySessionResponse {
  sessionId: string;
  wsUrl: string;
}

interface OpenSession {
  id: string;
  profileId: string;
  name: string;
  host: string;
  port: number;
  username: string;
  password: string;
  wsUrl: string;
  viewOnly: boolean;
  status: ConnectionState;
  desktopName: string;
  thumbnail?: string;
}

interface SessionHandle {
  captureThumbnail: () => string | undefined;
  disconnect: () => void;
  releaseInput: () => void;
  sendCtrlAltDel: () => void;
}

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}

function createId(value: string) {
  const slug = value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "");
  return slug || crypto.randomUUID();
}

async function createProxyUrl(host: string, port: number) {
  if (!isTauriRuntime()) {
    throw new Error("Desktop shell required. Browser preview does not expose raw VNC TCP.");
  }

  const proxy = await invoke<ProxySessionResponse>("create_proxy_session", { host, port });
  return proxy.wsUrl;
}

const SessionPane = forwardRef<
  SessionHandle,
  {
    session: OpenSession;
    active: boolean;
    onStateChange: (id: string, patch: Partial<OpenSession>) => void;
  }
>(function SessionPane({ session, active, onStateChange }, ref) {
  const vnc = useVncSession();
  const didConnect = useRef(false);

  useEffect(() => {
    if (didConnect.current) return;
    didConnect.current = true;
    vnc.connect({
      wsUrl: session.wsUrl,
      username: session.username,
      password: session.password,
      viewOnly: session.viewOnly,
    });
  }, [session.password, session.username, session.viewOnly, session.wsUrl, vnc]);

  useEffect(() => {
    vnc.setViewOnly(session.viewOnly);
  }, [session.viewOnly, vnc]);

  useEffect(() => {
    onStateChange(session.id, {
      status: vnc.state,
      desktopName: vnc.desktopName,
    });
  }, [onStateChange, session.id, vnc.desktopName, vnc.state]);

  useImperativeHandle(
    ref,
    () => ({
      captureThumbnail: vnc.captureThumbnail,
      disconnect: vnc.disconnect,
      releaseInput: vnc.releaseInput,
      sendCtrlAltDel: vnc.sendCtrlAltDel,
    }),
    [vnc],
  );

  return (
    <section className={`viewer-pane ${active ? "active" : ""}`}>
      <div className="viewer-surface" ref={vnc.targetRef} tabIndex={0} />
      {vnc.state !== "connected" && (
        <div className="empty-state">
          <Monitor size={32} />
          <strong>{vnc.state === "connecting" ? "Connecting" : session.name}</strong>
          <span>{vnc.lastError ?? `${session.host}:${session.port}`}</span>
        </div>
      )}
    </section>
  );
});

export function App() {
  const [connections, setConnections] = useState<SavedConnection[]>(() => {
    const stored = readConnections();
    return stored.length ? stored : defaultConnections;
  });
  const [name, setName] = useState(connections[0]?.name ?? "");
  const [host, setHost] = useState(connections[0]?.host ?? "127.0.0.1");
  const [port, setPort] = useState(String(connections[0]?.port ?? 5900));
  const [username, setUsername] = useState(connections[0]?.username ?? "");
  const [password, setPassword] = useState("");
  const [viewOnly, setViewOnly] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);
  const [openSessions, setOpenSessions] = useState<OpenSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [immersive, setImmersive] = useState(false);
  const sessionRefs = useRef<Record<string, SessionHandle | null>>({});
  const activeSessionIdRef = useRef<string | null>(null);

  const activeSession = useMemo(
    () => openSessions.find((session) => session.id === activeSessionId),
    [activeSessionId, openSessions],
  );
  const activeHandle = activeSessionId ? sessionRefs.current[activeSessionId] : null;

  useEffect(() => {
    activeSessionIdRef.current = activeSessionId;
  }, [activeSessionId]);

  useEffect(() => {
    if (!activeSession) return;
    setViewOnly(activeSession.viewOnly);
  }, [activeSession?.id]);

  useEffect(() => {
    const unlisteners = [
      listen("viewer://release-input", () => {
        const id = activeSessionIdRef.current;
        if (id) sessionRefs.current[id]?.releaseInput();
      }),
      listen("viewer://exit-fullscreen", () => {
        setImmersive(false);
        void invoke("set_fullscreen", { fullscreen: false }).catch(() => undefined);
      }),
      listen("viewer://disconnect-session", () => {
        const id = activeSessionIdRef.current;
        if (id) closeSession(id);
      }),
    ];

    return () => {
      unlisteners.forEach((unlisten) => {
        unlisten.then((dispose) => dispose()).catch(() => undefined);
      });
    };
  }, []);

  function selectConnection(connection: SavedConnection) {
    setName(connection.name);
    setHost(connection.host);
    setPort(String(connection.port));
    setUsername(connection.username);
    setPassword("");
    setConnectError(null);
  }

  const updateSession = useCallback((id: string, patch: Partial<OpenSession>) => {
    setOpenSessions((sessions) =>
      sessions.map((session) => (session.id === id ? { ...session, ...patch } : session)),
    );
  }, []);

  function saveProfile(thumbnail?: string) {
    const profileId = createId(`${name}-${host}-${port}`);
    const existing = connections.find((connection) => connection.id === profileId);
    const connection: SavedConnection = {
      id: profileId,
      name: name || `${host}:${port}`,
      host: host.trim(),
      port: Number(port),
      wsUrl: "",
      username,
      lastConnectedAt: new Date().toISOString(),
      thumbnail: thumbnail ?? existing?.thumbnail,
    };

    const next = upsertConnection(connection);
    setConnections(next);
    return connection;
  }

  async function startConnection() {
    setConnectError(null);
    const parsedPort = Number(port);
    if (!host.trim() || !Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535) {
      setConnectError("Enter a host and a port between 1 and 65535.");
      return;
    }

    try {
      const profile = saveProfile();
      const wsUrl = await createProxyUrl(profile.host, profile.port);
      const id = crypto.randomUUID();
      const nextSession: OpenSession = {
        id,
        profileId: profile.id,
        name: profile.name,
        host: profile.host,
        port: profile.port,
        username: profile.username,
        password,
        wsUrl,
        viewOnly,
        status: "connecting",
        desktopName: "Connecting",
        thumbnail: profile.thumbnail,
      };

      setOpenSessions((sessions) => [...sessions, nextSession]);
      setActiveSessionId(id);
      setPassword("");
    } catch (error) {
      setConnectError(error instanceof Error ? error.message : String(error));
    }
  }

  function closeSession(id: string) {
    sessionRefs.current[id]?.disconnect();
    sessionRefs.current[id] = null;
    setOpenSessions((sessions) => {
      const next = sessions.filter((session) => session.id !== id);
      if (activeSessionIdRef.current === id) {
        const nextActiveId = next.length ? next[next.length - 1].id : null;
        activeSessionIdRef.current = nextActiveId;
        setActiveSessionId(nextActiveId);
      }
      if (next.length === 0) setImmersive(false);
      return next;
    });
  }

  function closeActiveSession() {
    const id = activeSessionIdRef.current;
    if (id) closeSession(id);
  }

  function captureActiveThumbnail() {
    if (!activeSessionId || !activeSession) return;
    const thumbnail = activeHandle?.captureThumbnail();
    if (!thumbnail) return;

    updateSession(activeSessionId, { thumbnail });
    const nextConnections = connections.map((connection) =>
      connection.id === activeSession.profileId ? { ...connection, thumbnail } : connection,
    );
    setConnections(nextConnections);
    writeConnections(nextConnections);
  }

  async function toggleImmersive() {
    const next = !immersive;
    setImmersive(next);
    try {
      await invoke("set_fullscreen", { fullscreen: next });
    } catch {
      if (next) await document.documentElement.requestFullscreen?.();
      else await document.exitFullscreen?.();
    }
  }

  function toggleViewOnly(next: boolean) {
    setViewOnly(next);
    if (activeSessionId) updateSession(activeSessionId, { viewOnly: next });
  }

  return (
    <main className={`app-shell ${immersive ? "immersive" : ""}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">
            <MonitorUp size={20} />
          </div>
          <div>
            <strong>XVNCViewer</strong>
            <span>Local sessions</span>
          </div>
        </div>

        <section className="connect-panel">
          <label>
            Name
            <input value={name} onChange={(event) => setName(event.target.value)} />
          </label>
          <div className="host-row">
            <label>
              Host
              <input value={host} onChange={(event) => setHost(event.target.value)} />
            </label>
            <label>
              Port
              <input
                inputMode="numeric"
                value={port}
                onChange={(event) => setPort(event.target.value)}
              />
            </label>
          </div>
          <label>
            Username
            <input value={username} onChange={(event) => setUsername(event.target.value)} />
          </label>
          <label>
            Password
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="Current session only"
            />
          </label>
          {connectError && <div className="form-error">{connectError}</div>}
          <button className="primary-action" onClick={startConnection} type="button">
            <Play size={16} />
            Connect
          </button>
        </section>

        <section className="history-panel">
          <div className="panel-title">
            <History size={15} />
            Recent
          </div>
          <div className="history-list">
            {connections.map((connection) => (
              <button
                className="history-item"
                key={connection.id}
                onClick={() => selectConnection(connection)}
                type="button"
              >
                <span className="history-thumb">
                  {connection.thumbnail ? (
                    <img src={connection.thumbnail} alt="" />
                  ) : (
                    <Monitor size={18} />
                  )}
                </span>
                <span>
                  <strong>{connection.name}</strong>
                  <small>
                    {connection.host}:{connection.port}
                  </small>
                </span>
              </button>
            ))}
          </div>
        </section>
      </aside>

      <section className="workspace">
        <header className="session-bar">
          <div className="session-tabs" role="tablist" aria-label="Open VNC sessions">
            {openSessions.map((session) => (
              <button
                className={`session-tab ${session.id === activeSessionId ? "active" : ""}`}
                key={session.id}
                onClick={() => setActiveSessionId(session.id)}
                role="tab"
                type="button"
              >
                <span className={`status-dot ${session.status}`} />
                <span>
                  <strong>{session.name}</strong>
                  <small>
                    {session.host}:{session.port}
                  </small>
                </span>
                <span
                  className="close-tab"
                  onClick={(event) => {
                    event.stopPropagation();
                    closeSession(session.id);
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <X size={13} />
                </span>
              </button>
            ))}
            <button className="new-tab" onClick={() => setImmersive(false)} type="button">
              <Plus size={15} />
            </button>
          </div>

          <div className="toolbar">
            <button title="Clipboard" type="button">
              <Clipboard size={17} />
            </button>
            <button title="Send Ctrl+Alt+Del" onClick={activeHandle?.sendCtrlAltDel} type="button">
              <Command size={17} />
            </button>
            <button title="Release input" onClick={activeHandle?.releaseInput} type="button">
              <Keyboard size={17} />
            </button>
            <button title="Toggle fullscreen" onClick={toggleImmersive} type="button">
              <Fullscreen size={17} />
            </button>
            <button title="Disconnect current session" onClick={closeActiveSession} type="button">
              <Square size={15} />
            </button>
          </div>
        </header>

        <section className="viewer-frame">
          {openSessions.length === 0 && (
            <div className="empty-state">
              <Monitor size={34} />
              <strong>No active session</strong>
              <span>Choose a recent host or start a new connection.</span>
            </div>
          )}
          {openSessions.map((session) => (
            <SessionPane
              active={session.id === activeSessionId}
              key={session.id}
              onStateChange={updateSession}
              ref={(handle) => {
                sessionRefs.current[session.id] = handle;
              }}
              session={session}
            />
          ))}
        </section>

        <footer className="session-strip">
          <div className="session-thumbs">
            {openSessions.map((session) => (
              <button
                className={`session-thumb ${session.id === activeSessionId ? "active" : ""}`}
                key={session.id}
                onClick={() => setActiveSessionId(session.id)}
                type="button"
              >
                {session.thumbnail ? <img src={session.thumbnail} alt="" /> : <Monitor size={18} />}
                <span>{session.name}</span>
              </button>
            ))}
          </div>
          <div className="viewer-toggles">
            <button onClick={captureActiveThumbnail} type="button">
              Snapshot
            </button>
            <label className="switch">
              <input
                checked={viewOnly}
                onChange={(event) => toggleViewOnly(event.target.checked)}
                type="checkbox"
              />
              View only
            </label>
          </div>
        </footer>
      </section>
    </main>
  );
}
