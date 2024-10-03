use bigdecimal::BigDecimal;
use ethers::providers::{Provider, Ws};
use fatal::fatal;
use serde::{Deserialize, Serialize};
use std::{
    str::FromStr,
    sync::{mpsc::Sender, Arc},
    thread::{self, sleep},
    time::{Duration, Instant, SystemTime},
};
use uuid::Uuid;

use crate::stats::{ExecStatus, TimerExecutorStats};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlashLiquidityParams {
    pub token: String,
    pub price: BigDecimal,
    pub slippage: BigDecimal,
    pub time_limit: Duration,
}

pub struct LaminatedProxyListener {
    ws: String,
    executor_frame: TimerExecutorFrame,
}

impl LaminatedProxyListener {
    pub fn new(ws: String, executor_frame: TimerExecutorFrame) -> LaminatedProxyListener {
        LaminatedProxyListener { ws, executor_frame }
    }

    pub async fn listen(&mut self) {
        println!("Starting listener...");
        println!(
            "Connecting to the provider with URL {} ...",
            self.ws.as_str()
        );
        match Provider::<Ws>::connect(self.ws.as_str()).await {
            Ok(provider) => {
                println!("Connected successfully!");
                let _client = Arc::new(provider);
                // TODO: Create a contract from ABI

                // Here is a simulation of the LaminatedProxy triggering and running executors.
                let params1 = FlashLiquidityParams {
                    token: "USDC".into(),
                    price: BigDecimal::from(2500),
                    slippage: BigDecimal::from_str("0.5").unwrap(),
                    time_limit: Duration::new(2 * 60, 0),
                };
                self.executor_frame.start_executor(params1);

                sleep(Duration::new(1, 0));

                let params = FlashLiquidityParams {
                    token: "USDC".into(),
                    price: BigDecimal::from(2502),
                    slippage: BigDecimal::from_str("0.35").unwrap(),
                    time_limit: Duration::new(60, 0),
                };
                self.executor_frame.start_executor(params);

                sleep(Duration::new(60, 0));

                let params = FlashLiquidityParams {
                    token: "USDT".into(),
                    price: BigDecimal::from(2503),
                    slippage: BigDecimal::from_str("0.31").unwrap(),
                    time_limit: Duration::new(1 * 60, 0),
                };
                self.executor_frame.start_executor(params);

                sleep(Duration::new(15, 0));

                let params = FlashLiquidityParams {
                    token: "USDT".into(),
                    price: BigDecimal::from(2680),
                    slippage: BigDecimal::from_str("0.99").unwrap(),
                    time_limit: Duration::new(25, 0),
                };
                self.executor_frame.start_executor(params);

                sleep(Duration::new(24 * 60 * 60, 0));
            }
            Err(err) => {
                fatal!("Failed connection to the chain: {}", err);
            }
        }
    }
}

// The executor combined with a timer, PoC version.
// For real prod version the timer is to be moved into its own thread to reduce a number of
// contract read calls.
struct TimerExecutor {
    // Unique ID, used for monitoring
    id: Uuid,

    // Creation time since Unix epoch, used for ordering executors in stats
    creation_time: Duration,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,
}

impl TimerExecutor {
    pub fn new(tick_duration: Duration, stats_tx: Sender<TimerExecutorStats>) -> TimerExecutor {
        let creation_time_res = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
        if creation_time_res.is_err() {
            fatal!(
                "Error getting system time: {}",
                creation_time_res.err().unwrap()
            );
        }
        let ret = TimerExecutor {
            id: Uuid::new_v4(),
            creation_time: creation_time_res.ok().unwrap(),
            tick_duration,
            stats_tx,
        };

        ret
    }

    // Execute the FlashLiquidity executor with given params.
    pub fn execute(&self, params: FlashLiquidityParams) {
        // Initialize timer
        let now = Instant::now();
        while now.elapsed() < params.time_limit {
            // Actions

            // Push stats
            self.send_stats(ExecStatus::RUNNING, &now, params.clone());

            // Wait for the next tick
            sleep(self.tick_duration);
        }
        // Sending post-exec stats
        self.send_stats(ExecStatus::TIMEOUT, &now, params);
    }

    // Send statistics into the stats channel
    fn send_stats(&self, status: ExecStatus, now: &Instant, params: FlashLiquidityParams) {
        let remaining;
        if status == ExecStatus::RUNNING {
            remaining = params.time_limit.abs_diff(now.elapsed());
        } else {
            remaining = Duration::new(0, 0);
        }
        let res = self.stats_tx.send(TimerExecutorStats {
            id: self.id,
            creation_time: self.creation_time,
            status,
            params: params.clone(),
            elapsed: now.elapsed(),
            remaining,
        });
        if let Some(err) = res.err() {
            println!("Error sending stats: {}", err);
        }
    }
}

// The executor frame. It's a container for running executors
pub struct TimerExecutorFrame {
    // Duration of time ticks
    tick_duration: Duration,

    // Stats channels
    stats_tx: Sender<TimerExecutorStats>,
}

impl TimerExecutorFrame {
    pub fn new(secs: u64, nanos: u32, stats_tx: Sender<TimerExecutorStats>) -> TimerExecutorFrame {
        let ret = TimerExecutorFrame {
            tick_duration: Duration::new(secs, nanos),
            stats_tx,
        };

        ret
    }

    pub fn start_executor(&mut self, params: FlashLiquidityParams) {
        let dur = self.tick_duration.clone();
        let executor = TimerExecutor::new(dur, self.stats_tx.clone());
        thread::spawn(move || {
            executor.execute(params);
        });
    }
}
