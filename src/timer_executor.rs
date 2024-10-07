use ethers::providers::Middleware;
use fatal::fatal;
use std::{
    sync::{mpsc::Sender, Arc},
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};
use threadpool::ThreadPool;
use uuid::Uuid;

use crate::{
    contracts_abi::{
        call_breaker::CallBreaker,
        laminator::{AdditionalData, ProxyPushedFilter},
    },
    solver_factory::SolverFactory,
    stats::{ExecStatus, TimerExecutorStats},
};

// The executor combined with a timer, PoC version.
// For real prod version the timer is to be moved into its own thread to reduce a number of
// contract read calls.
struct TimerRequestExecutor<M> {
    // Unique ID, used for monitoring
    id: Uuid,

    // Call breaker contract
    call_breaker_contract: Arc<CallBreaker<M>>,

    // Creation time since Unix epoch, used for ordering executors in stats
    creation_time: Duration,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,
}

impl<M: Middleware> TimerRequestExecutor<M> {
    pub fn new(
        call_breaker_contract: Arc<CallBreaker<M>>,
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
            call_breaker_contract,
            creation_time: creation_time_res.ok().unwrap(),
            tick_duration,
            stats_tx,
        };

        ret
    }

    // Execute the FlashLiquidity executor with given params.
    pub fn execute(&self, event: ProxyPushedFilter) {
        println!("Executor {} started", self.id);
        // Initialize timer
        let now = Instant::now();
        // Create a solver of a given type
        let solver = SolverFactory::new_solver(event.selector.into(), &event.data_values);
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
        match solver.time_limit() {
            Ok(time_limit) => {
                while now.elapsed() < time_limit {
                    // Actions
                    match solver.exec_solver_step() {
                        Ok(succeeded) => {
                            if succeeded {
                                // contract.verify(event.call_objs, returns_bytes, associated_data, hintdices);
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
                            print!("Error in solver: {}", err);
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
            Err(err) => {
                print!("Error getting time limit: {}", err);
            }
        }
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
    // Call breaker contract
    call_breaker_contract: Arc<CallBreaker<M>>,

    // Duration of time ticks
    tick_duration: Duration,

    // Stats channels
    stats_tx: Sender<TimerExecutorStats>,

    // Executors Pool
    pool: ThreadPool,
}

impl<M: Middleware + 'static> TimerExecutorFrame<M> {
    pub fn new(
        call_breaker_contract: Arc<CallBreaker<M>>,
        tick_secs: u64,
        tick_nanos: u32,
        stats_tx: Sender<TimerExecutorStats>,
        n_workers: usize,
    ) -> TimerExecutorFrame<M> {
        let ret = TimerExecutorFrame {
            call_breaker_contract,
            tick_duration: Duration::new(tick_secs, tick_nanos),
            stats_tx,
            pool: ThreadPool::new(n_workers),
        };

        ret
    }

    pub fn start_executor(&self, event: ProxyPushedFilter) {
        let dur = self.tick_duration.clone();
        let executor = TimerRequestExecutor::new(
            self.call_breaker_contract.clone(),
            dur,
            self.stats_tx.clone(),
        );
        let exec_id = executor.id.clone();
        self.pool.execute(move || {
            executor.execute(event);
        });
        println!(
            "New executor {} pushed to pool, active: {}, queued: {}, panicked: {}",
            exec_id,
            self.pool.active_count(),
            self.pool.queued_count(),
            self.pool.panic_count()
        );
    }
}
