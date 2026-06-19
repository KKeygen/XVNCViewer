mod proxy;

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use uuid::Uuid;

#[derive(Default)]
struct WindowSessionState(Mutex<HashMap<String, WindowSessionEntry>>);

struct FocusedWindowState(Mutex<String>);

const WINDOW_SESSION_TTL: Duration = Duration::from_secs(45);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenSessionWindowRequest {
    name: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    view_only: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowSessionPayload {
    name: String,
    host: String,
    port: u16,
    username: String,
    password: String,
    view_only: bool,
    ws_url: String,
}

struct WindowSessionEntry {
    payload: WindowSessionPayload,
    created_at: Instant,
}

#[tauri::command]
fn set_fullscreen(window: WebviewWindow, fullscreen: bool) -> Result<(), String> {
    window
        .set_fullscreen(fullscreen)
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn open_session_window(
    app: AppHandle,
    proxy_state: State<'_, proxy::ProxyState>,
    window_state: State<'_, WindowSessionState>,
    request: OpenSessionWindowRequest,
) -> Result<(), String> {
    let proxy = proxy_state
        .create_session(request.host.clone(), request.port)
        .await?;
    let token = Uuid::new_v4().to_string();
    let payload = WindowSessionPayload {
        name: request.name.clone(),
        host: request.host,
        port: request.port,
        username: request.username,
        password: request.password,
        view_only: request.view_only,
        ws_url: proxy.ws_url,
    };

    {
        let mut sessions = window_state
            .0
            .lock()
            .map_err(|_| "failed to lock window session state".to_string())?;
        remove_expired_window_sessions(&mut sessions);
        sessions.insert(
            token.clone(),
            WindowSessionEntry {
                payload,
                created_at: Instant::now(),
            },
        );
    }

    let label = format!("session-{token}");
    let url = format!("index.html?windowSession={token}");
    if let Err(err) = WebviewWindowBuilder::new(&app, label, WebviewUrl::App(url.into()))
        .title(request.name)
        .inner_size(1200.0, 780.0)
        .min_inner_size(720.0, 480.0)
        .resizable(true)
        .build()
    {
        let mut sessions = window_state
            .0
            .lock()
            .map_err(|_| "failed to lock window session state".to_string())?;
        sessions.remove(&token);
        return Err(format!("failed to open session window: {err}"));
    }

    Ok(())
}

#[tauri::command]
fn claim_window_session(
    window: WebviewWindow,
    window_state: State<'_, WindowSessionState>,
    token: String,
) -> Result<WindowSessionPayload, String> {
    let expected_label = format!("session-{token}");
    if window.label() != expected_label {
        return Err("window is not authorized to claim this session".to_string());
    }

    let mut sessions = window_state
        .0
        .lock()
        .map_err(|_| "failed to lock window session state".to_string())?;
    remove_expired_window_sessions(&mut sessions);

    sessions
        .remove(&token)
        .map(|entry| entry.payload)
        .ok_or_else(|| "window session not found or already claimed".to_string())
}

fn remove_expired_window_sessions(sessions: &mut HashMap<String, WindowSessionEntry>) {
    sessions.retain(|_, entry| entry.created_at.elapsed() <= WINDOW_SESSION_TTL);
}

fn emit_to_focused_window(app: &AppHandle, event: &str) {
    let label = app
        .state::<FocusedWindowState>()
        .0
        .lock()
        .map(|label| label.clone())
        .unwrap_or_else(|_| "main".to_string());

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.emit(event, ());
    }
}

fn register_shortcuts(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let release_input = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
        Code::KeyZ,
    );
    let exit_fullscreen = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
        Code::KeyX,
    );
    let disconnect_session = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
        Code::KeyQ,
    );

    app.global_shortcut()
        .on_shortcut(release_input, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                emit_to_focused_window(_app, "viewer://release-input");
            }
        })?;

    app.global_shortcut()
        .on_shortcut(exit_fullscreen, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                emit_to_focused_window(_app, "viewer://exit-fullscreen");
            }
        })?;

    app.global_shortcut()
        .on_shortcut(disconnect_session, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                emit_to_focused_window(_app, "viewer://disconnect-session");
            }
        })?;

    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(proxy::ProxyState::new())
        .manage(WindowSessionState::default())
        .manage(FocusedWindowState(Mutex::new("main".to_string())))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Focused(true)) {
                if let Ok(mut label) = window.state::<FocusedWindowState>().0.lock() {
                    *label = window.label().to_string();
                }
            }

            if matches!(event, WindowEvent::Destroyed) {
                if let Some(token) = window.label().strip_prefix("session-") {
                    if let Ok(mut sessions) = window.state::<WindowSessionState>().0.lock() {
                        sessions.remove(token);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_fullscreen,
            open_session_window,
            claim_window_session,
            proxy::create_proxy_session,
            proxy::proxy_status
        ])
        .setup(|app| {
            if let Err(err) = register_shortcuts(app) {
                eprintln!("failed to register global shortcuts: {err}");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running XVNCViewer");
}
