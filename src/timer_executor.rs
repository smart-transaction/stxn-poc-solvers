use ethers::{providers::Middleware, types::U256};
use fatal::fatal;
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    sync::{mpsc::Sender, Mutex},
    task::JoinSet,
    time::sleep,
};
use uuid::Uuid;

use crate::{
    contracts_abi::laminator::{AdditionalData, ProxyPushedFilter},
    solver::{Solver, SolverParams},
    solvers::limit_order::LimitOrderSolver,
    stats::{Status, TimerExecutorStats, TransactionStatus},
};

// The executor combined with a timer, PoC version.
// For real prod version the timer is to be moved into its own thread to reduce a number of
// contract read calls.
struct TimerRequestExecutor<M: Clone> {
    // Unique ID, used for monitoring
    id: Uuid,

    // Params that are used in solver.
    params: SolverParams<M>,

    // Creation time since Unix epoch, used for ordering executors in stats
    creation_time: Duration,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,
}

impl<M: Middleware + Clone + 'static> TimerRequestExecutor<M> {
    pub fn new(
        params: SolverParams<M>,
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
            params,
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
        let solver = LimitOrderSolver::new(event.clone(), self.params.clone());
        if let Err(err) = &solver {
            println!("Error on creating a solver: {}", err);
            self.send_stats(
                event.sequence_number,
                String::new(),
                Status::Failed,
                TransactionStatus::NotExecuted,
                err.to_string(),
                &Duration::new(0, 0),
                &now,
                &event.data_values,
            )
            .await;
            return;
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
        let mut last_transaction_status = TransactionStatus::NotExecuted;
        while now.elapsed() < time_limit {
            // Actions
            match solver.exec_solver_step().await {
                Ok(succeeded) => {
                    if succeeded {
                        match solver.final_exec().await {
                            Ok(succeeded) => {
                                if succeeded {
                                    self.send_stats(
                                        event.sequence_number,
                                        solver.app(),
                                        Status::Succeeded,
                                        TransactionStatus::Succeeded,
                                        String::new(),
                                        &time_limit,
                                        &now,
                                        &event.data_values,
                                    )
                                    .await;
                                    println!("Executor {} successfully finished", self.id);
                                    return;
                                } else {
                                    self.send_stats(
                                        event.sequence_number,
                                        solver.app(),
                                        Status::Running,
                                        TransactionStatus::TransactionPending,
                                        String::new(),
                                        &time_limit,
                                        &now,
                                        &event.data_values,
                                    )
                                    .await;
                                    last_transaction_status = TransactionStatus::TransactionPending;
                                }
                            }
                            Err(err) => {
                                println!("Error in solver final exec: {}", err);
                                self.send_stats(
                                    event.sequence_number,
                                    solver.app(),
                                    Status::Running,
                                    TransactionStatus::TransactionFailed,
                                    err.to_string(),
                                    &time_limit,
                                    &now,
                                    &event.data_values,
                                )
                                .await;
                                last_transaction_status = TransactionStatus::TransactionFailed;
                            }
                        }
                    } else {
                        self.send_stats(
                            event.sequence_number,
                            solver.app(),
                            Status::Running,
                            TransactionStatus::StepPending,
                            String::new(),
                            &time_limit,
                            &now,
                            &event.data_values,
                        )
                        .await;
                        last_transaction_status = TransactionStatus::StepPending;
                    }
                }
                Err(err) => {
                    println!("Error in solver step call: {}", err);
                    self.send_stats(
                        event.sequence_number,
                        solver.app(),
                        Status::Failed,
                        TransactionStatus::StepFailed,
                        err.to_string(),
                        &time_limit,
                        &now,
                        &event.data_values,
                    )
                    .await;
                    last_transaction_status = TransactionStatus::StepFailed;
                }
            }
            // Wait for the next tick
            sleep(self.tick_duration).await;
        }
        // Sending post-exec stats
        self.send_stats(
            event.sequence_number,
            solver.app(),
            Status::Timeout,
            last_transaction_status,
            String::new(),
            &time_limit,
            &now,
            &event.data_values,
        )
        .await;
        println!("Executor {} finished by timeout", self.id);
    }

    // Send statistics into the stats channel
    async fn send_stats(
        &self,
        sequence_number: U256,
        app: String,
        status: Status,
        transaction_status: TransactionStatus,
        message: String,
        time_limit: &Duration,
        now: &Instant,
        params: &Vec<AdditionalData>,
    ) {
        let remaining;
        if status == Status::Running {
            remaining = time_limit.abs_diff(now.elapsed());
        } else {
            remaining = Duration::new(0, 0);
        }
        let res = self
            .stats_tx
            .send(TimerExecutorStats {
                id: self.id,
                sequence_number: sequence_number.as_u32(),
                app,
                creation_time: self.creation_time,
                status,
                transaction_status,
                message,
                params: params.clone(),
                elapsed: now.elapsed(),
                remaining,
            })
            .await;
        if let Some(err) = res.err() {
            println!("Error sending stats: {}", err);
        }
    }
}

// The executor frame. It's a container for running executors
pub struct TimerExecutorFrame<M: Clone> {
    solver_params: SolverParams<M>,

    // Join set for parallel executing
    exec_set: Arc<Mutex<JoinSet<()>>>,

    // Duration of time ticks
    tick_duration: Duration,

    // Stats channels
    stats_tx: Sender<TimerExecutorStats>,
}

impl<M: Middleware + Clone + 'static> TimerExecutorFrame<M> {
    pub fn new(
        solver_params: SolverParams<M>,
        exec_set: Arc<Mutex<JoinSet<()>>>,
        tick_secs: u64,
        tick_nanos: u32,
        stats_tx: Sender<TimerExecutorStats>,
    ) -> TimerExecutorFrame<M> {
        let ret = TimerExecutorFrame {
            solver_params,
            exec_set,
            tick_duration: Duration::new(tick_secs, tick_nanos),
            stats_tx,
        };

        ret
    }

    pub async fn start_executor(&self, event: ProxyPushedFilter) {
        let dur = self.tick_duration.clone();
        let executor =
            TimerRequestExecutor::new(self.solver_params.clone(), dur, self.stats_tx.clone());
        let exec_id = executor.id.clone();
        let mut exec_set = self.exec_set.lock().await;
        exec_set.spawn(async move {
            executor.execute(event).await;
        });
        println!(
            "New executor {} is spawned, tasks running: {}",
            exec_id,
            exec_set.len(),
        );
    }
}
