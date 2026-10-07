mod ai;
mod commands;
mod database;
mod observation;
mod panel;
mod websearch;
mod window;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, RunEvent,
};
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Init logging — visible in `npm run tauri dev` terminal
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("projectz=debug,info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .try_init();
    tracing::info!("Rust logging initialized");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let engine_dir = app_data_dir.join("engines");
            std::fs::create_dir_all(&engine_dir)?;
            std::fs::create_dir_all(&app_data_dir)?;
            let database = std::sync::Arc::new(
                database::Database::open(app_data_dir.join("projectz.sqlite3"))
                    .map_err(std::io::Error::other)?,
            );
            app.manage(std::sync::Arc::clone(&database));
            let runtime = ai::runtime::AiRuntime::new_with_database(
                engine_dir,
                std::sync::Arc::clone(&database),
            );
            // Before the window exists, so the first reply a user sends already
            // resolves against the folder they picked last time rather than the
            // app's launch directory.
            runtime.restore_workspace();
            app.manage(runtime);
            // The panel browser's history, installed before anything can ask for
            // a browser so the first `panel_browser_open` finds live state rather
            // than reporting a page it then forgets.
            panel::browser::install(app.handle());

            let show = MenuItem::with_id(app, "show", "Show ProjectZ", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        if let Some(runtime) = app.try_state::<ai::runtime::AiRuntime>() {
                            runtime.local_models().unload();
                        }
                        // The tray's Quit bypasses `CloseRequested`, so the
                        // window's own close handler never runs. Writing here
                        // is what stops a tray-quit from losing the geometry.
                        if let Some(database) =
                            app.try_state::<std::sync::Arc<database::Database>>()
                        {
                            window::persist_on_exit(app, database.inner());
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                // Geometry first, while the window is still hidden: it has never
                // been painted, so the user sees it arrive once, already in the
                // right place, instead of at the configured default and then
                // jumping. `restore` always ends in `show()`, so a window it
                // cannot work out for still opens rather than staying invisible.
                window::restore(&window, &database);
                // Then start tracking moves and resizes. Installed after the
                // restore so the geometry we just applied is not immediately
                // written back as though the user had chosen it.
                window::attach(&window, std::sync::Arc::clone(&database));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ai_chat,
            commands::ai_cancel_chat,
            commands::ai_skip_local_reasoning,
            commands::ai_answer_approval,
            commands::ai_set_viewed_session,
            commands::ai_generate_title,
            commands::skills_list,
            commands::skills_enabled,
            commands::skills_create,
            commands::skills_update,
            commands::skills_set_enabled,
            commands::skills_delete,
            commands::database_initialize,
            commands::database_frontend_imported,
            commands::database_import_frontend,
            commands::database_list_sessions,
            commands::database_list_session_headers,
            commands::database_list_subagents,
            commands::ai_running_subagents,
            commands::database_list_session_messages,
            commands::database_save_session,
            commands::database_delete_session,
            commands::database_clear_sessions,
            commands::database_get_setting,
            commands::database_set_setting,
            commands::database_tables,
            commands::database_table_rows,
            commands::database_usage_report,
            commands::workspaces_list,
            commands::workspace_open,
            commands::workspace_select,
            commands::workspace_close,
            commands::read_chat_attachment,
            commands::extract_pdf_attachment,
            commands::ai_list_endpoints,
            commands::provider_favicon,
            commands::ai_add_endpoint,
            commands::ai_update_endpoint,
            commands::ai_test_endpoint,
            commands::ai_model_capabilities,
            commands::ai_set_model_enabled,
            commands::ai_provider_capabilities,
            commands::ai_remove_endpoint,
            commands::local_engines_list,
            commands::local_engines_install,
            commands::local_engines_uninstall,
            commands::local_engines_custom_install,
            commands::local_model_engine_set,
            commands::local_models_list,
            commands::local_models_add,
            commands::local_models_remove,
            commands::local_model_select,
            commands::local_runtime_settings,
            commands::local_runtime_estimate,
            commands::local_device_memory,
            commands::local_kv_quantisations,
            commands::local_path_exists,
            commands::local_runtime_settings_save,
            commands::local_model_status,
            commands::local_model_logs,
            commands::local_model_load,
            commands::local_model_unload,
            panel::fs::panel_fs_list,
            panel::fs::panel_read_preview,
            panel::fs::panel_write_file,
            panel::fs::panel_create_file,
            panel::fs::panel_create_folder,
            panel::fs::panel_move_file,
            panel::fs::panel_rename_file,
            panel::fs::panel_delete_file,
            panel::terminal::panel_terminal_run,
            panel::browser::panel_browser_navigate,
            panel::browser::panel_browser_open_tab,
            panel::browser::panel_browser_select_tab,
            panel::browser::panel_browser_close_tab,
            panel::browser::panel_browser_set_bounds,
            panel::browser::panel_browser_set_visible,
            panel::browser::panel_browser_reload,
            panel::browser::panel_browser_back,
            panel::browser::panel_browser_forward,
            panel::browser::panel_browser_close,
        ])
        .build(tauri::generate_context!())
        .expect("error while building application")
        .run(|app, event| {
            if matches!(event, RunEvent::Exit) {
                if let Some(runtime) = app.try_state::<ai::runtime::AiRuntime>() {
                    runtime.local_models().unload();
                }
                // A final write for every exit path that reaches here without
                // passing through the tray menu -- a normal quit, or a window
                // closed by the OS at session end.
                if let Some(database) = app.try_state::<std::sync::Arc<database::Database>>() {
                    window::persist_on_exit(app, database.inner());
                }
            }
        });
}
