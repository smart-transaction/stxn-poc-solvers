use std::{collections::HashMap, sync::Arc};

use axum::{extract::State, response::Json};

use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    account: Address,
    amount: U256,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportStats {
    accounts: usize,
    total_amount: U256,
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

pub async fn get_reports_stats(
    reports: State<Arc<Mutex<HashMap<Address, U256>>>>,
) -> Json<ReportStats> {
    let reports = reports.lock().await;
    let total = reports
        .iter()
        .fold(U256::zero(), |acc, v| acc + *v.1);

    Json(ReportStats {
        accounts: reports.len(),
        total_amount: total,
    })
}
