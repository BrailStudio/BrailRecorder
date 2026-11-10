use brail_core::events::AppEvent;
use tauri::{AppHandle, Emitter};

/// Every backend-to-frontend notification goes through this one Tauri
/// event name, tagged by `AppEvent`'s own `#[serde(tag = "type")]` — see
/// `brail-core::events` for why one channel beats one-event-per-message
/// (a single frontend listener, one place to log every event for
/// diagnostics, no risk of the frontend missing a rarely-used channel it
/// forgot to subscribe to).
pub const EVENT_CHANNEL: &str = "brail://event";

pub fn emit(app: &AppHandle, event: AppEvent) {
    if let Err(e) = app.emit(EVENT_CHANNEL, &event) {
        tracing::error!("failed to emit event to frontend: {e}");
    }
}
