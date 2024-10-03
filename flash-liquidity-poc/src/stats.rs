use crate::flash_liquidity;
use fatal::fatal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::Receiver,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;
use warp::reply::{json, Json};

// Executor statistics
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum ExecStatus {
    RUNNING,
    COMPLETED,
    TIMEOUT,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimerExecutorStats {
    pub id: Uuid,
    pub creation_time: Duration,
    pub status: ExecStatus,
    pub params: flash_liquidity::FlashLiquidityParams,
    pub elapsed: Duration,
    pub remaining: Duration,
}

pub fn get_stats_json(
    stats: Arc<Mutex<HashMap<Uuid, TimerExecutorStats>>>,
    filter: HashSet<ExecStatus>,
) -> Json {
    match stats.lock() {
        Ok(stats) => {
            let mut filtered = stats
                .clone()
                .into_values()
                .filter(|el| filter.is_empty() || filter.contains(&el.status))
                .collect::<Vec<TimerExecutorStats>>();
            filtered.sort_by(|el1, el2| el1.creation_time.cmp(&el2.creation_time));
            json(&filtered)
        }
        Err(err) => {
            println!("Error locking the stats map: {}", err);
            json(&"".to_string())
        }
    }
}

pub fn run_stats_receive(
    rx: &Receiver<TimerExecutorStats>,
    stats_map: Arc<Mutex<HashMap<Uuid, TimerExecutorStats>>>,
) {
    loop {
        match rx.recv() {
            Ok(stats) => match stats_map.lock() {
                Ok(mut stats_map) => {
                    stats_map.insert(stats.id, stats);
                }
                Err(err) => {
                    fatal!("Error locking the mutex: {}", err);
                }
            },
            Err(err) => {
                println!("Error receiving stats from the channel: {}", err);
            }
        }
    }
}
