//! Periodic maintenance, the equivalent of vaultwarden's job scheduler.

use std::time::Duration;

use crate::{
    CONFIG,
    api::{AppState, purge_expired_sends},
    config::RuntimeSettings,
    db::models::{Attachment, Device, Event, Tombstone},
    util::now,
};

const INTERVAL: Duration = Duration::from_secs(60 * 60);

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        loop {
            ticker.tick().await;
            run_once(&state).await;
        }
    });
}

async fn run_once(state: &AppState) {
    match purge_expired_sends(state).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("purged {n} expired sends"),
        Err(e) => tracing::warn!("send purge failed: {e}"),
    }

    let event_cutoff = now() - chrono::Duration::days(RuntimeSettings::current().event_retention_days);
    let tombstone_cutoff = now() - chrono::Duration::days(CONFIG.tombstone_retention_days);
    let pending_cutoff = now() - chrono::Duration::hours(24);
    let device_cutoff = now() - chrono::Duration::days(CONFIG.refresh_token_days);
    let result = state
        .db
        .write(move |conn| {
            let events = Event::purge_older_than(conn, event_cutoff)?;
            let tombstones = Tombstone::purge_older_than(conn, tombstone_cutoff)?;
            let blobs = Attachment::purge_stale_pending(conn, pending_cutoff)?;
            let devices = Device::purge_idle(conn, device_cutoff)?;
            Ok((events, tombstones, blobs, devices))
        })
        .await;
    match result {
        Ok((events, tombstones, blobs, devices)) => {
            if events + tombstones + blobs.len() + devices > 0 {
                tracing::info!(
                    "maintenance: {events} events, {tombstones} tombstones, {} stale uploads, {devices} idle devices removed",
                    blobs.len()
                );
            }
            state.storage.delete_many(blobs).await;
        }
        Err(e) => tracing::warn!("maintenance failed: {e}"),
    }

    if let Err(e) = state.storage.cleanup_tmp(Duration::from_secs(24 * 60 * 60)).await {
        tracing::warn!("tmp cleanup failed: {e}");
    }
    state.login_limiter.cleanup();
}
