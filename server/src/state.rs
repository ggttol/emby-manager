use crate::settings::Settings;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub settings: Settings,
    pub task_slots: Arc<Semaphore>,
    pub clouddrive_slot: Arc<Semaphore>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(pool: PgPool, settings: Settings) -> Self {
        Self {
            task_slots: Arc::new(Semaphore::new(settings.task_concurrency)),
            clouddrive_slot: Arc::new(Semaphore::new(1)),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(45))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            pool,
            settings,
        }
    }
}
