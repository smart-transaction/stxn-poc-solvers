use ethers::{
    abi::Address,
    providers::{Middleware, StreamExt},
    types::{BlockNumber, U256},
};
use fatal::fatal;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc::Sender, Mutex},
    task::JoinSet,
};

use crate::{
    contracts_abi::{CallPushedFilter, LaminatedProxy, SolverData},
    solver::SolverParams,
    solvers::cleanapp_scheduler::CleanAppSchedulerSolver,
    stats::TimerExecutorStats,
    timer_executor::TimerRequestExecutor,
};

pub struct LaminatorListener<M: Clone> {
    // The address of the laminator contract.
    laminated_proxy_address: Address,

    // KITN disbursement scheduler address.
    kitn_disbursement_scheduler_address: Address,

    // The middleware to be used
    middleware: Arc<M>,

    // Mapping of app selectors to solver params.
    solver_params: SolverParams<M>,

    // JoinSet for using for executors spawning.
    exec_set: Arc<Mutex<JoinSet<()>>>,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,

    // CleanApp reports pool
    reports_pool: Arc<Mutex<HashMap<Address, U256>>>,

    // Temporaty stores the cron string from the event
    params: Vec<SolverData>,
}

impl<M: Middleware + Clone + 'static> LaminatorListener<M> {
    pub fn new(
        laminated_proxy_address: Address,
        kitn_disbursement_scheduler_address: Address,
        middleware: Arc<M>,
        solver_params: SolverParams<M>,
        exec_set: Arc<Mutex<JoinSet<()>>>,
        tick_duration: Duration,
        stats_tx: Sender<TimerExecutorStats>,
        reports_pool: Arc<Mutex<HashMap<Address, U256>>>,
    ) -> LaminatorListener<M> {
        LaminatorListener::<M> {
            laminated_proxy_address,
            kitn_disbursement_scheduler_address,
            middleware,
            solver_params,
            exec_set,
            tick_duration,
            stats_tx,
            reports_pool,
            params: Vec::new(),
        }
    }

    fn is_cleanapp_event(&self, event: &CallPushedFilter) -> bool {
        if event.call_objs.len() != 3 {
            return false;
        }
        return event.call_objs[0].addr == self.kitn_disbursement_scheduler_address;
    }

    pub async fn listen(&mut self) {
        let laminated_proxy_contract =
            LaminatedProxy::new(self.laminated_proxy_address, self.middleware.clone());
        let events = laminated_proxy_contract
            .event::<CallPushedFilter>()
            .from_block(BlockNumber::Latest);
        loop {
            match events.stream().await {
                Ok(stream) => {
                    let mut stream_take = stream.take(10);
                    println!("Listening the event CallPushed ...");
                    while let Some(Ok(mut call_pushed)) = stream_take.next().await {
                        if !self.is_cleanapp_event(&call_pushed) {
                            continue;
                        }
                        let mut exec_set = self.exec_set.lock().await;
                        let tick_duration = self.tick_duration.clone();
                        let stats_tx = self.stats_tx.clone();
                        let reports_pool = self.reports_pool.clone();
                        let solver_params = self.solver_params.clone();
                        let laminated_proxy_address = self.laminated_proxy_address;
                        let kitn_disbursement_scheduler_address =
                            self.kitn_disbursement_scheduler_address;

                        let mut cron = String::new();
                        if !call_pushed.data.is_empty() {
                            for ad in &call_pushed.data {
                                match ad.name.as_str() {
                                    "CRON" => {
                                        cron = ad.value.clone();
                                    }
                                    &_ => {}
                                }
                            }
                            if !cron.is_empty() {
                                self.params = call_pushed.data.clone();
                            }
                        } else {
                            call_pushed.data = self.params.clone();
                            for ad in &call_pushed.data {
                                match ad.name.as_str() {
                                    "CRON" => {
                                        cron = ad.value.clone();
                                    }
                                    &_ => {}
                                }
                            }
                        }
                        if !cron.is_empty() {
                            exec_set.spawn(async move {
                                match CleanAppSchedulerSolver::new(
                                    call_pushed.clone(),
                                    solver_params,
                                    laminated_proxy_address,
                                    kitn_disbursement_scheduler_address,
                                    reports_pool,
                                    cron,
                                ) {
                                    Ok(clean_app_scheduler_solver) => {
                                        let executor = TimerRequestExecutor::<
                                            CleanAppSchedulerSolver<M>,
                                        >::new(
                                            clean_app_scheduler_solver,
                                            tick_duration,
                                            stats_tx,
                                        );
                                        executor.execute(call_pushed).await;
                                    }
                                    Err(err) => {
                                        println!("Error creating the solver: {}", err);
                                    }
                                }
                            });
                        }
                    }
                }
                Err(err) => {
                    fatal!("Error reading events from stream: {}", err);
                }
            }
        }
    }
}
