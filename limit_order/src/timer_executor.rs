use ethers::types::U256;
use fatal::fatal;
use std::time::{Duration, SystemTime};
use tokio::{sync::mpsc::Sender, time::{sleep, Instant}};
use uuid::Uuid;

use crate::{
    contracts_abi::laminator::{AdditionalData, ProxyPushedFilter},
    solver::Solver,
    stats::{Status, TimerExecutorStats, TransactionStatus},
};

// The executor combined with a timer, PoC version.
// For real prod version the timer is to be moved into its own thread to reduce a number of
// contract read calls.
pub struct TimerRequestExecutor<S> {
    // The solver
    solver: S,

    // Unique ID, used for monitoring
    id: Uuid,

    // Creation time since Unix epoch, used for ordering executors in stats
    creation_time: Duration,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,
}

impl<S: Solver> TimerRequestExecutor<S> {
    pub fn new(
        solver: S,
        tick_duration: Duration,
        stats_tx: Sender<TimerExecutorStats>,
    ) -> TimerRequestExecutor<S> {
        let creation_time_res = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
        if creation_time_res.is_err() {
            fatal!(
                "Error getting system time: {}",
                creation_time_res.err().unwrap()
            );
        }
        let ret = TimerRequestExecutor {
            solver,
            id: Uuid::new_v4(),
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
        if self.solver.time_limit().is_err() {
            print!(
                "Error getting time limit: {}",
                &self.solver.time_limit().err().unwrap()
            );
            return;
        }
        // Tokens reading.
        let time_limit = self.solver.time_limit().ok().unwrap();
        let mut last_transaction_status = TransactionStatus::NotExecuted;
        let mut last_message = String::new();
        while now.elapsed() < time_limit {
            // Actions
            match self.solver.exec_solver_step().await {
                Ok(response) => {
                    last_message = response.message.clone();
                    if response.succeeded {
                        self.send_stats(
                            event.sequence_number,
                            self.solver.app(),
                            Status::Running,
                            TransactionStatus::TransactionPending,
                            response.message.clone(),
                            &time_limit,
                            &now,
                            &event.data_values,
                        )
                        .await;
                        match self.solver.final_exec().await {
                            Ok(response) => {
                                last_message = response.message.clone();
                                if response.succeeded {
                                    self.send_stats(
                                        event.sequence_number,
                                        self.solver.app(),
                                        Status::Succeeded,
                                        TransactionStatus::Succeeded,
                                        response.message.clone(),
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
                                        self.solver.app(),
                                        Status::Running,
                                        TransactionStatus::TransactionPending,
                                        response.message.clone(),
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
                                    self.solver.app(),
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
                            self.solver.app(),
                            Status::Running,
                            TransactionStatus::StepPending,
                            response.message.clone(),
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
                        self.solver.app(),
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
            self.solver.app(),
            Status::Timeout,
            last_transaction_status,
            last_message,
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
