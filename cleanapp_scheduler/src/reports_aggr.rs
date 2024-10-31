use std::{collections::HashMap, sync::Arc};

use axum::Json;
use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    account: Address,
    amount: U256,
}

pub async fn aggregate_report(
    Json(body): Json<Report>,
    reports: Arc<Mutex<HashMap<Address, U256>>>,
) {
    println!("Report: {:#?}", body);
    let mut reports = reports.lock().await;
    match reports.get_mut(&body.account) {
        Some(amount) => {
            *amount += body.amount;
        }
        None => {
            reports.insert(body.account, body.amount);
        }
    }
    println!("{:#?}", reports);
}
