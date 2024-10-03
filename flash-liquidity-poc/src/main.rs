use clap::Parser;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
};
use tokio::task::JoinSet;
use warp::Filter;

mod flash_liquidity;
mod stats;

use flash_liquidity::{LaminatedProxyListener, TimerExecutorFrame};
use stats::{get_stats_json, run_stats_receive, ExecStatus, TimerExecutorStats};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(long, default_value_t = 1)]
    pub tick_secs: u64,

    #[arg(long, default_value_t = 0)]
    pub tick_nanos: u32,

    #[arg(long)]
    pub ws: String,
}

#[tokio::main]
async fn main() {
    // Get args
    let args = Args::parse();

    let stats_map = Arc::new(Mutex::new(HashMap::new()));
    let (stats_tx, stats_rx): (Sender<TimerExecutorStats>, Receiver<TimerExecutorStats>) =
        mpsc::channel();
    let mut exec_set = JoinSet::new();

    let exec_frame = TimerExecutorFrame::new(args.tick_secs, args.tick_nanos, stats_tx);
    let mut listener = LaminatedProxyListener::new(args.ws, exec_frame);
    exec_set.spawn(async move {
        listener.listen().await;
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
