use clap::Parser;
use ethers::{
    core::types::Address, middleware::SignerMiddleware, providers::{Middleware, Provider, Ws}, signers::{LocalWallet, Signer}
};
use fatal::fatal;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
};
use tokio::task::JoinSet;
use warp::Filter;

mod contracts_abi;
mod laminator_listener;
mod solver_factory;
mod solvers;
mod stats;
mod timer_executor;

use crate::contracts_abi::{CallBreaker, Laminator};

use laminator_listener::LaminatedProxyListener;
use stats::{get_stats_json, run_stats_receive, ExecStatus, TimerExecutorStats};
use timer_executor::TimerExecutorFrame;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(long)]
    pub chain_id: u64,

    #[arg(long)]
    pub ws_chain_url: String,

    #[arg(long)]
    pub laminator_address: Address,

    #[arg(long)]
    pub call_breaker_address: Address,

    #[arg(long)]
    pub wallet_private_key: LocalWallet,

    #[arg(long, default_value_t = 1)]
    pub tick_secs: u64,

    #[arg(long, default_value_t = 0)]
    pub tick_nanos: u32,

    #[arg(long, default_value_t = 5)]
    pub n_executors: usize,
}

#[tokio::main]
async fn main() {
    // Get args
    let args = Args::parse();
    let wallet = args.wallet_private_key.with_chain_id(args.chain_id);
    let stats_map = Arc::new(Mutex::new(HashMap::new()));
    let (stats_tx, stats_rx): (
        Sender<TimerExecutorStats>,
        Receiver<TimerExecutorStats>,
    ) = mpsc::channel();
    let mut exec_set = JoinSet::new();

    println!(
        "Connecting to the provider with URL {} ...",
        args.ws_chain_url.as_str()
    );
    let provider_res = Provider::<Ws>::connect(args.ws_chain_url.as_str()).await;
    if provider_res.is_err() {
        fatal!("Failed connection to the chain: {}", provider_res.err().unwrap());
    }
    println!("Connected successfully!");
    let ws_client = Arc::new(SignerMiddleware::new(provider_res.ok().unwrap(), wallet));

    let laminator_contract = Laminator::new(args.laminator_address, ws_client.clone());
    let call_breaker_contract = Arc::new(CallBreaker::new(args.call_breaker_address, ws_client.clone()));

    let exec_frame =
        TimerExecutorFrame::new(call_breaker_contract, args.tick_secs, args.tick_nanos, stats_tx, args.n_executors);
    
    let mut listener = LaminatedProxyListener::new(laminator_contract, exec_frame);

    let block_res = ws_client.provider().get_block_number().await;
    if block_res.is_err() {
        fatal!("Error getting block: {}", block_res.err().unwrap());
    }
    let block = block_res.ok().unwrap();

    exec_set.spawn(async move {
        listener.listen(block).await;
    });
    let stats_map_copy = Arc::clone(&stats_map);
    exec_set.spawn(async move {
        run_stats_receive(&stats_rx, stats_map_copy);
    });
    let default_route = warp::path::end().map(|| warp::reply::html("FlashLiquidity Solver"));
    let stats = warp::path("stats").map(move || {
        let stats_map = Arc::clone(&stats_map);
        let mut filter = HashSet::new();
        filter.insert(ExecStatus::RUNNING);
        get_stats_json(stats_map, filter)
    });
    let routes = default_route.or(stats);

    // Start all services
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}
