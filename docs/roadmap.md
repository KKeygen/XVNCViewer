# Roadmap

## Phase 1: Local Viewer Shell

- Tauri 2 desktop shell using system WebViews.
- React noVNC viewer surface.
- Saved connection history in localStorage.
- Built-in loopback WebSocket-to-TCP proxy.
- Multiple open VNC session tabs.
- Remote-control fullscreen mode that hides app chrome around the active session.
- Native fullscreen command.
- Moonlight-style escape shortcuts:
  - `Ctrl+Alt+Shift+Z`: release or toggle input capture.
  - `Ctrl+Alt+Shift+X`: exit or toggle fullscreen.
  - `Ctrl+Alt+Shift+Q`: disconnect the current session.

## Phase 2: Native Transport Hardening

noVNC requires WebSocket transport, while VNC servers expose raw TCP. The desktop
app owns this bridge so users do not need to run `websockify` manually.

Implemented baseline:

- Rust async runtime with `tokio`.
- Local loopback listener with an ephemeral port.
- WebSocket server with `tokio-tungstenite`.
- One-time per-session route such as `/vnc/:session_id?secret=...`.
- Handshake timeout, idle timeout, pending handshake limit, and active bridge limit.
- Native command creates a session:

```text
create_proxy_session({ host, port }) -> ws://127.0.0.1:<port>/vnc/<session_id>?secret=...
```

Remaining hardening:

- Add Rust unit/integration tests for handshake, secret validation, and byte forwarding.
- Add target allowlist policy for enterprise deployments.
- Add richer proxy telemetry for pending, active, closed, and failed sessions.
- Keep WebSocket proxy URLs hidden from the primary UI; expose only in a future
  debug panel if needed.

## Phase 3: Viewer Features

- Better session thumbnails and background session health indicators.
- Thumbnail sampling with noVNC `toDataURL()` or `toBlob()`.
- Clipboard bridge through noVNC plus native system clipboard.
- Credential storage through the platform keychain rather than frontend storage.
- Raw keyboard work:
  - Keep WebView keyboard events for ordinary text and shortcuts.
  - Use native hooks only for capture modes that require them.
  - Preserve escape shortcuts at shell level.

## Release Targets

- macOS Apple Silicon: Tauri `.app`/`.dmg`.
- Windows x64: Tauri installer with WebView2 runtime handling.
