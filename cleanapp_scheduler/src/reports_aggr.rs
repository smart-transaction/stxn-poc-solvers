use std::{collections::HashMap, sync::Arc};

use axum::Json;
use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    cron: String,
    account: Address,
    amount: U256,
}

pub async fn aggregate_report(
    Json(body): Json<Report>,
    reports: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>>,
) {
    let mut reports = reports.lock().await;
    match reports.get_mut(&body.cron)  {
        Some(cron_reports) => {
          match cron_reports.get_mut(&body.account) {
            Some(amount) => {
              *amount += body.amount;
            }
            None => {
              cron_reports.insert(body.account, body.amount);
            }
          }
        }
        None => {
          let mut cron_reports = HashMap::new();
          cron_reports.insert(body.account, body.amount);
          reports.insert(body.cron.clone(), cron_reports);
        }
    }
}
