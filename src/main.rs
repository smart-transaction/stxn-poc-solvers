use axum::{routing::{get, Router}, serve};
use clap::Parser;
use ethers::{
    core::types::Address,
    middleware::MiddlewareBuilder,
    providers::{Middleware, Provider, Ws},
    signers::{LocalWallet, Signer},
};
use fatal::fatal;
use std::{
    collections::HashMap,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::{mpsc::{self, Receiver, Sender}, Mutex}, task::JoinSet};

use crate::laminator_listener::LaminatorListener;
use crate::stats::{get_stats_json, run_stats_receive, TimerExecutorStats};
use crate::timer_executor::TimerExecutorFrame;

mod contracts_abi;
mod laminator_listener;
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
    pub flash_loan_address: Address,

    #[arg(long)]
    pub swap_pool_address: Address,

    #[arg(long)]
    pub wallet_private_key: LocalWallet,

    #[arg(long, default_value_t = 1)]
    pub tick_secs: u64,

    #[arg(long, default_value_t = 0)]
    pub tick_nanos: u32,
}

#[tokio::main]
async fn main() {
    // Get args
    let args = Args::parse();
    let wallet = args.wallet_private_key.with_chain_id(args.chain_id);
    let stats_map = Arc::new(Mutex::new(HashMap::new()));
    let (stats_tx, mut stats_rx): (Sender<TimerExecutorStats>, Receiver<TimerExecutorStats>) =
        mpsc::channel(100);
    let exec_set = Arc::new(Mutex::new(JoinSet::new()));

    println!(
        "Connecting to the chain with URL {} ...",
        args.ws_chain_url.as_str()
    );
    let provider_res = Provider::<Ws>::connect(args.ws_chain_url.as_str()).await;
    if provider_res.is_err() {
        fatal!(
            "Failed connection to the chain: {}",
            provider_res.err().unwrap()
        );
    }
    println!("Connected successfully!");

    let wallet_address = wallet.address();
    let provider = Arc::new(
        provider_res
            .ok()
            .unwrap()
            .with_signer(wallet)
    );

    // Addresses of specific solvers contracts.
    let mut custom_contracts_addresses: HashMap<String, Address> = HashMap::new();
    custom_contracts_addresses.insert("FLASH_LOAN".to_string(), args.flash_loan_address);
    custom_contracts_addresses.insert("SWAP_POOL".to_string(), args.swap_pool_address);

    let exec_frame = TimerExecutorFrame::new(
        args.call_breaker_address,
        wallet_address,
        provider.clone(),
        custom_contracts_addresses,
        exec_set.clone(),
        args.tick_secs,
        args.tick_nanos,
        stats_tx.clone(),
    );

    let mut listener = LaminatorListener::new(args.laminator_address, provider.clone(), exec_frame);

    let block_res = provider.provider().get_block_number().await;
    if block_res.is_err() {
        fatal!("Error getting block: {}", block_res.err().unwrap());
    }
    let block = block_res.ok().unwrap();

    let stats_map_copy = Arc::clone(&stats_map);

    // Axum setup

    let app = Router::new()
        .route("/", get(|| async { "Smart Transactions Solver" }))
        .route("/stats/limit_order", get(get_stats_json))
        .with_state(stats_map);

    let tcp_listener = TcpListener::bind(format!("0.0.0.0:{}", args.port)).await.unwrap();
    // Start all services
    println!("Starting server at port {}", args.port);

    {
        let mut exec_set = exec_set.lock().await;
        exec_set.spawn(async move {
            listener.listen(block).await;
        });
        exec_set.spawn(async move {
            run_stats_receive(&mut stats_rx, stats_map_copy).await;
        });
    };
    serve(tcp_listener, app).await.unwrap();
}
