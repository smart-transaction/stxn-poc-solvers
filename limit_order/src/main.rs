use axum::{
    routing::{get, Router},
    serve,
};
use clap::Parser;
use ethers::{
    core::types::Address,
    middleware::MiddlewareBuilder,
    providers::{Provider, Ws},
    signers::{LocalWallet, Signer},
};
use fatal::fatal;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    task::JoinSet,
};

use crate::{
    call_breaker_listener::CallBreakerListener,
    solver::{selector, SolverParams},
    solvers::limit_order::{APP_SELECTOR, FLASH_LOAN_NAME, SWAP_POOL_NAME},
    stats::{get_stats_json, run_stats_receive, TimerExecutorStats},
};

mod call_breaker_listener;
mod contracts_abi;
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
    pub call_breaker_address: Address,

    #[arg(long)]
    pub flash_loan_address: Address,

    #[arg(long)]
    pub swap_pool_address: Address,

    #[arg(long)]
    pub limit_order_wallet_private_key: LocalWallet,

    #[arg(long, default_value_t = 1)]
    pub tick_secs: u64,

    #[arg(long, default_value_t = 0)]
    pub tick_nanos: u32,
}

#[tokio::main]
async fn main() {
    // Get args
    let args = Args::parse();
    let limit_order_wallet = args
        .limit_order_wallet_private_key
        .with_chain_id(args.chain_id);
    let limit_order_wallet_address = limit_order_wallet.address();
    let stats_map = Arc::new(Mutex::new(HashMap::new()));
    let (stats_tx, mut stats_rx): (Sender<TimerExecutorStats>, Receiver<TimerExecutorStats>) =
        mpsc::channel(100);
    let exec_set = Arc::new(Mutex::new(JoinSet::new()));

    println!(
        "Connecting to the chain with URL {} ...",
        args.ws_chain_url.as_str()
    );
    let limit_order_provider = Provider::<Ws>::connect(args.ws_chain_url.as_str()).await;
    if limit_order_provider.is_err() {
        fatal!(
            "Failed connection to the chain: {}",
            limit_order_provider.err().unwrap()
        );
    }
    println!("Connected successfully!");

    let limit_order_provider = Arc::new(
        limit_order_provider
            .ok()
            .unwrap()
            .with_signer(limit_order_wallet.clone()),
    );

    // Addresses of specific solvers contracts.
    let mut custom_contracts_addresses: HashMap<String, Address> = HashMap::new();
    custom_contracts_addresses.insert(FLASH_LOAN_NAME.to_string(), args.flash_loan_address);
    custom_contracts_addresses.insert(SWAP_POOL_NAME.to_string(), args.swap_pool_address);

    let mut solver_params = HashMap::new();
    solver_params.insert(
        selector(APP_SELECTOR.to_string()),
        SolverParams {
            call_breaker_address: args.call_breaker_address,
            solver_address: limit_order_wallet_address,
            middleware: limit_order_provider.clone(),
            extra_contract_addresses: custom_contracts_addresses.clone(),
            guard: Arc::new(Mutex::new(true)),
            wallet: limit_order_wallet,
        },
    );

    let mut listener = CallBreakerListener::new(
        args.call_breaker_address,
        limit_order_provider.clone(),
        solver_params,
        exec_set.clone(),
        Duration::new(args.tick_secs, args.tick_nanos),
        stats_tx.clone(),
    );
    let stats_map_copy = Arc::clone(&stats_map);

    // Axum setup
    let app = Router::new()
        .route("/", get(|| async { "Smart Transactions Solver" }))
        .route("/stats/limit_order", get(get_stats_json))
        .with_state(stats_map);

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
