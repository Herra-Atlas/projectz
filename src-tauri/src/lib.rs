mod ai;
mod commands;
mod database;
mod jobs;
mod observation;
mod panel;
mod websearch;
mod window;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent,
};
use tracing_subscriber::EnvFilter;

/// The tray's jobs item, held in app state.
///
/// Kept here so anything that pauses or resumes the scheduler can bring the menu
/// up to date, not only a click on the menu itself: the Jobs screen's own Pause all
/// and the tray are two doors into one switch, and a menu that only knows about its
/// own door tells the user the wrong thing.
#[derive(Clone)]
pub struct JobsToggle(pub CheckMenuItem<tauri::Wry>);

/// Puts the scheduler's paused state on the tray item.
pub fn sync_jobs_toggle(app: &tauri::AppHandle, paused: bool) {
    let Some(item) = app.try_state::<JobsToggle>() else {
        return;
    };
    let _ = item.0.set_checked(!paused);
    let _ = item.0.set_text(if paused { "Jobs paused" } else { "Jobs running" });
}

/// Brings the main window up and in front.
///
/// Shared by the tray's Show item and by double-clicking the icon, so both arrive
/// the same way -- a window minimised from the taskbar comes back rather than being
/// shown while still minimised.
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

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
            // Started before the runtime is managed, so the scheduler task holds
            // its own clone of it rather than reaching back into app state: a run
            // it starts needs the runtime while a command may be using the same
            // entry, and a clone is what keeps those two from borrowing one.
            let scheduler = jobs::Scheduler::start(app.handle().clone(), runtime.clone());
            app.manage(runtime);
            app.manage(scheduler);
            // The panel browser's history, installed before anything can ask for
            // a browser so the first `panel_browser_open` finds live state rather
            // than reporting a page it then forgets.
            panel::browser::install(app.handle());

            let show = MenuItem::with_id(app, "show", "Show ProjectZ", true, None::<&str>)?;
            // A check item that states the situation rather than the action: the
            // label reads "Jobs running" or "Jobs paused", the tick shows which,
            // and clicking flips it. An item labelled with what it *does* leaves
            // the reader to work out what is true now.
            let jobs_item =
                CheckMenuItem::with_id(app, "jobs", "Jobs running", true, true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show,
                    &PredefinedMenuItem::separator(app)?,
                    &jobs_item,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;
            let mut tray = TrayIconBuilder::new().menu(&menu);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            // The menu is on the right button and the left one opens the window, the
            // way a tray icon is normally read. Left click opening the menu as well
            // would mean a double click never reaches the window at all: the first
            // click would raise a menu over it.
            let jobs_toggle = jobs_item.clone();
            app.manage(JobsToggle(jobs_toggle.clone()));
            tray.show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => show_main(app),
                    "jobs" => {
                        let Some(scheduler) = app.try_state::<jobs::Scheduler>() else {
                            return;
                        };
                        let pause = !scheduler.is_paused();
                        scheduler.set_paused(pause);
                        sync_jobs_toggle(app, pause);
                        // The Jobs screen reads its state from the database, so it
                        // needs telling that the switch moved -- and this is the same
                        // event a run emits, which it already listens for.
                        let _ = app.emit("job-event", serde_json::json!({ "status": "paused", "paused": pause }));
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
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
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
            commands::job_list,
            commands::job_create,
            commands::job_update,
            commands::job_delete,
            commands::job_set_enabled,
            commands::job_runs,
            commands::job_run_now,
            commands::job_scheduler_status,
            commands::job_scheduler_pause,
            commands::job_machine_totals,
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
