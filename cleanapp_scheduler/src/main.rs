use axum::{
    routing::{get, post, Router},
    serve,
};
use clap::Parser;
use ethers::{
    core::types::Address,
    middleware::MiddlewareBuilder,
    providers::{Provider, Ws},
    signers::{LocalWallet, Signer}, types::U256,
};
use fatal::fatal;
use reports_aggr::aggregate_report;
use solver::{selector, SolverParams};
use solvers::cleanapp_scheduler;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    task::JoinSet,
};

use crate::laminator_listener::LaminatorListener;
use crate::stats::{get_stats_json, run_stats_receive, TimerExecutorStats};

mod contracts_abi;
mod laminator_listener;
mod reports_aggr;
mod solver;
mod solvers;
mod stats;
mod timer_executor;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(long, default_value_t = 3030)]
    pub port: u16,

    #[arg(long)]
    pub chain_id: u64,

    #[arg(long)]
    pub ws_chain_url: String,

    #[arg(long)]
    pub laminator_address: Address,

    #[arg(long)]
    pub call_breaker_address: Address,

    #[arg(long)]
    pub cleanapp_wallet_private_key: LocalWallet,

    #[arg(long, default_value_t = 1)]
    pub tick_secs: u64,

    #[arg(long, default_value_t = 0)]
    pub tick_nanos: u32,
}

#[tokio::main]
async fn main() {
    // Get args
    let args = Args::parse();
    let cleanapp_wallet = args
        .cleanapp_wallet_private_key
        .with_chain_id(args.chain_id);
    let stats_map = Arc::new(Mutex::new(HashMap::new()));
    let (stats_tx, mut stats_rx): (Sender<TimerExecutorStats>, Receiver<TimerExecutorStats>) =
        mpsc::channel(100);
    let exec_set = Arc::new(Mutex::new(JoinSet::new()));
    let reports_pool: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>> = Arc::new(Mutex::new(HashMap::new()));

    println!(
        "Connecting to the chain with URL {} ...",
        args.ws_chain_url.as_str()
    );
    let cleanapp_provider = Provider::<Ws>::connect(args.ws_chain_url.as_str()).await;
    if cleanapp_provider.is_err() {
        fatal!(
            "Failed connection to the chain: {}",
            cleanapp_provider.err().unwrap()
        );
    }
    println!("Connected successfully!");

    let cleanapp_wallet_address = cleanapp_wallet.address();
    let cleanapp_provider = Arc::new(
        cleanapp_provider
            .ok()
            .unwrap()
            .with_signer(cleanapp_wallet),
    );

    let mut solver_params = HashMap::new();
    solver_params.insert(
        selector(cleanapp_scheduler::APP_SELECTOR.to_string()),
        SolverParams {
            call_breaker_address: args.call_breaker_address,
            solver_address: cleanapp_wallet_address,
            middleware: cleanapp_provider.clone(),
            extra_contract_addresses: HashMap::new(),
            guard: Arc::new(Mutex::new(true)),
        },
    );

    let mut listener = LaminatorListener::new(
        args.laminator_address,
        cleanapp_provider.clone(),
        solver_params,
        exec_set.clone(),
        Duration::new(args.tick_secs, args.tick_nanos),
        stats_tx.clone(),
        reports_pool.clone(),
    );
    let stats_map_copy = Arc::clone(&stats_map);

    // Axum setup
    let app = Router::new()
        .route("/", get(|| async { "Smart Transactions Solver" }))
        .route("/stats/cleanapp", get(get_stats_json))
        .with_state(stats_map)
        .route("/get_report", post({
            let shared_state = Arc::clone(&reports_pool);
            move |body| aggregate_report(body, shared_state)
        }),
    );

    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{}", args.port))
        .await
        .unwrap();
    // Start all services
    println!("Starting server at port {}", args.port);

    {
        let mut exec_set = exec_set.lock().await;
        exec_set.spawn(async move {
            listener.listen().await;
        });
        exec_set.spawn(async move {
            run_stats_receive(&mut stats_rx, stats_map_copy).await;
        });
    };
    serve(tcp_listener, app).await.unwrap();
}
