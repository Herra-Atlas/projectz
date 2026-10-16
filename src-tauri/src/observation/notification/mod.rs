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

/// A reply that failed, for a window the user is not looking at.
///
/// **Not gated on the notification preference, unlike the success above.** The
/// preference exists so a finished reply does not interrupt the user; a reply that
/// *died* while they were elsewhere is the opposite case -- the one worth
/// interrupting for, because there is nothing else on screen to tell them. It is
/// still suppressed for the conversation that is open, where the transcript shows
/// the failure itself.
pub fn notify_run_failed(app: &AppHandle, session_title: Option<&str>, reason: &str) {
    let title = session_title
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let reason = truncate(reason.trim(), 160);
    let body = match title {
        Some(title) => format!("A response in \"{}\" failed: {reason}", truncate(title, 80)),
        None => format!("A response failed: {reason}"),
    };
    if let Err(error) = app
        .notification()
        .builder()
        .title("ProjectZ")
        .body(body)
        .show()
    {
        warn!("failure notification failed: {error}");
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
