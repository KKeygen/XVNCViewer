mod proxy;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[tauri::command]
fn set_fullscreen(app: AppHandle, fullscreen: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    window
        .set_fullscreen(fullscreen)
        .map_err(|err| err.to_string())
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

    let handle = app.handle().clone();
    app.global_shortcut()
        .on_shortcut(release_input, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                let _ = handle.emit("viewer://release-input", ());
            }
        })?;

    let handle = app.handle().clone();
    app.global_shortcut()
        .on_shortcut(exit_fullscreen, move |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_fullscreen(false);
                }
                let _ = handle.emit("viewer://exit-fullscreen", ());
            }
        })?;

    let handle = app.handle().clone();
    app.global_shortcut()
        .on_shortcut(disconnect_session, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                let _ = handle.emit("viewer://disconnect-session", ());
            }
        })?;

    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(proxy::ProxyState::new())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            set_fullscreen,
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
