use axum::{extract::State, response::Json};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::Receiver, Mutex};
use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

use crate::contracts_abi::SolverData;

// Executor statistics
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Running,
    Succeeded,
    Failed,
    Timeout,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TransactionStatus {
    Succeeded,
    StepFailed,
    TransactionFailed,
    StepPending,
    TransactionPending,
    NotExecuted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimerExecutorStats {
    pub id: Uuid,
    pub sequence_number: u32,
    pub app: String,
    pub creation_time: Duration,
    pub status: Status,
    pub transaction_status: TransactionStatus,
    pub message: String,
    pub params: Vec<SolverData>,
    pub elapsed: Duration,
    pub remaining: Duration,
}

pub async fn get_stats_json(
    stats: State<Arc<Mutex<HashMap<Uuid, TimerExecutorStats>>>>,
) -> Json<Vec<TimerExecutorStats>> {
    let stats = stats.lock().await;
    let mut filtered = stats
        .clone()
        .into_values()
        .collect::<Vec<TimerExecutorStats>>();
    filtered.sort_by(|el1, el2| el1.creation_time.cmp(&el2.creation_time));
    Json(filtered)
}

pub async fn run_stats_receive(
    rx: &mut Receiver<TimerExecutorStats>,
    stats_map: Arc<Mutex<HashMap<Uuid, TimerExecutorStats>>>,
) {
    while let Some(stats) = rx.recv().await {
        let mut stats_map = stats_map.lock().await;
        stats_map.insert(stats.id, stats);
    }
}
