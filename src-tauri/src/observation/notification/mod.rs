use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tracing::warn;

pub fn notify_response_completed(app: &AppHandle, session_title: Option<&str>) {
    let title = session_title
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let body = match title {
        Some(title) => format!("Response ready in \"{}\".", truncate(title, 80)),
        None => "A response is ready.".to_string(),
    };
    if let Err(error) = app
        .notification()
        .builder()
        .title("ProjectZ")
        .body(body)
        .show()
    {
        warn!("response notification failed: {error}");
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        format!("{truncated}…")
    } else {
        truncated
    }
}
