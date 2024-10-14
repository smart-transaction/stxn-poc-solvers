use ethers::{abi::Address, providers::Middleware};
use fatal::fatal;
use std::{
    collections::HashMap,
    sync::{mpsc::Sender, Arc, Mutex},
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::{
    contracts_abi::laminator::{AdditionalData, ProxyPushedFilter},
    solvers::limit_order::LimitOrderSolver,
    stats::{ExecStatus, TimerExecutorStats},
};

// The executor combined with a timer, PoC version.
// For real prod version the timer is to be moved into its own thread to reduce a number of
// contract read calls.
struct TimerRequestExecutor<M> {
    // Unique ID, used for monitoring
    id: Uuid,

    // Middleware instance
    middleware: Arc<M>,

    // Call Breaker Address
    call_breaker_address: Address,

    // The address of the walled that is used by the ws_client.
    solver_address: Address,

    // Custom contract addresses
    custom_contracts_addresses: HashMap<String, Address>,

    // Creation time since Unix epoch, used for ordering executors in stats
    creation_time: Duration,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,
}

impl<M: Middleware + 'static> TimerRequestExecutor<M> {
    pub fn new(
        call_breaker_address: Address,
        solver_address: Address,
        middleware: Arc<M>,
        custom_contracts_addresses: HashMap<String, Address>,
        tick_duration: Duration,
        stats_tx: Sender<TimerExecutorStats>,
    ) -> TimerRequestExecutor<M> {
        let creation_time_res = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
        if creation_time_res.is_err() {
            fatal!(
                "Error getting system time: {}",
                creation_time_res.err().unwrap()
            );
        }
        let ret = TimerRequestExecutor {
            id: Uuid::new_v4(),
            middleware,
            call_breaker_address,
            solver_address,
            custom_contracts_addresses,
            creation_time: creation_time_res.ok().unwrap(),
            tick_duration,
            stats_tx,
        };

        ret
    }

    // Execute the FlashLiquidity executor with given params.
    pub async fn execute(&self, event: ProxyPushedFilter) {
        println!("Executor {} started", self.id);
        // Initialize timer
        let now = Instant::now();
        // Create a solver of a given type
        let solver = LimitOrderSolver::new(
            &event,
            self.call_breaker_address,
            self.solver_address,
            &self.custom_contracts_addresses,
            self.middleware.clone(),
        );
        if let Err(err) = &solver {
            self.send_stats(
                String::new(),
                ExecStatus::FAILED,
                err.to_string(),
                &Duration::new(0, 0),
                &now,
                &event.data_values,
            );
        }
        let solver = solver.ok().unwrap();
        if solver.time_limit().is_err() {
            print!(
                "Error getting time limit: {}",
                &solver.time_limit().err().unwrap()
            );
            return;
        }
        // Tokens reading.
        let time_limit = solver.time_limit().ok().unwrap();
        while now.elapsed() < time_limit {
            // Actions
            match solver.exec_solver_step().await {
                Ok(succeeded) => {
                    if succeeded {
                        match solver.final_exec().await {
                            Ok(succeeded) => {
                                if succeeded {
                                    self.send_stats(
                                        solver.app(),
                                        ExecStatus::SUCCEEDED,
                                        String::new(),
                                        &time_limit,
                                        &now,
                                        &event.data_values,
                                    );
                                    break;
                                }
                            }
                            Err(err) => {
                                println!("Error in solver final exec: {}", err);
                                self.send_stats(
                                    solver.app(),
                                    ExecStatus::FAILED,
                                    err.to_string(),
                                    &time_limit,
                                    &now,
                                    &event.data_values,
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    println!("Error in solver step call: {}", err);
                    self.send_stats(
                        solver.app(),
                        ExecStatus::FAILED,
                        err.to_string(),
                        &time_limit,
                        &now,
                        &event.data_values,
                    );
                }
            }

            // Push stats
            self.send_stats(
                solver.app(),
                ExecStatus::RUNNING,
                String::new(),
                &time_limit,
                &now,
                &event.data_values,
            );

            // Wait for the next tick
            sleep(self.tick_duration);
        }
        // Sending post-exec stats
        self.send_stats(
            solver.app(),
            ExecStatus::TIMEOUT,
            String::new(),
            &time_limit,
            &now,
            &event.data_values,
        );
        println!("Executor {} finished", self.id);
    }

    // Send statistics into the stats channel
    fn send_stats(
        &self,
        app: String,
        status: ExecStatus,
        message: String,
        time_limit: &Duration,
        now: &Instant,
        params: &Vec<AdditionalData>,
    ) {
        let remaining;
        if status == ExecStatus::RUNNING {
            remaining = time_limit.abs_diff(now.elapsed());
        } else {
            remaining = Duration::new(0, 0);
        }
        let res = self.stats_tx.send(TimerExecutorStats {
            id: self.id,
            app,
            creation_time: self.creation_time,
            status,
            message,
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
pub struct TimerExecutorFrame<M> {
    // Call breaker contract address
    call_breaker_address: Address,

    // The address provided by the wallet used in ws_client
    solver_address: Address,

    // Middleware instance
    middleware: Arc<M>,

    // Custom contract addresses
    custom_contracts_addresses: HashMap<String, Address>,

    // Join set for parallel executing
    exec_set: Arc<Mutex<JoinSet<()>>>,

    // Duration of time ticks
    tick_duration: Duration,

    // Stats channels
    stats_tx: Sender<TimerExecutorStats>,
}

impl<M: Middleware + 'static> TimerExecutorFrame<M> {
    pub fn new(
        call_breaker_address: Address,
        solver_address: Address,
        middleware: Arc<M>,
        custom_contracts_addresses: HashMap<String, Address>,
        exec_set: Arc<Mutex<JoinSet<()>>>,
        tick_secs: u64,
        tick_nanos: u32,
        stats_tx: Sender<TimerExecutorStats>,
    ) -> TimerExecutorFrame<M> {
        let ret = TimerExecutorFrame {
            call_breaker_address,
            solver_address,
            middleware,
            custom_contracts_addresses,
            exec_set,
            tick_duration: Duration::new(tick_secs, tick_nanos),
            stats_tx,
        };

        ret
    }

    pub fn start_executor(&self, event: ProxyPushedFilter) {
        let dur = self.tick_duration.clone();
        let executor = TimerRequestExecutor::new(
            self.call_breaker_address,
            self.solver_address,
            self.middleware.clone(),
            self.custom_contracts_addresses.clone(),
            dur,
            self.stats_tx.clone(),
        );
        let exec_id = executor.id.clone();
        match self.exec_set.lock() {
            Ok(mut exec_set) => {
                exec_set.spawn(async move {
                    executor.execute(event).await;
                });
                println!(
                    "New executor {} is spawned, tasks running: {}",
                    exec_id,
                    exec_set.len(),
                );
            }
            Err(err) => {
                println!("Starting executor {} failed: {}", exec_id, err);
            }
        }
    }
}
