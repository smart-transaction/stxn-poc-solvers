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
    contracts_abi::{CallPushedFilter, LaminatedProxy},
    solver::SolverParams,
    solvers::cleanapp_scheduler::CleanAppSchedulerSolver,
    stats::TimerExecutorStats,
    timer_executor::TimerRequestExecutor,
};

pub struct LaminatorListener<M: Clone> {
    // The address of the laminator contract.
    laminated_proxy_address: Address,

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
    reports_pool: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>>,
}

impl<M: Middleware + Clone + 'static> LaminatorListener<M> {
    pub fn new(
        laminated_proxy_address: Address,
        middleware: Arc<M>,
        solver_params: SolverParams<M>,
        exec_set: Arc<Mutex<JoinSet<()>>>,
        tick_duration: Duration,
        stats_tx: Sender<TimerExecutorStats>,
        reports_pool: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>>,
    ) -> LaminatorListener<M> {
        LaminatorListener::<M> {
            laminated_proxy_address,
            middleware,
            solver_params,
            exec_set,
            tick_duration,
            stats_tx,
            reports_pool,
        }
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
                    println!("Listening the event ProxyPushed ...");
                    while let Some(Ok(call_pushed)) = stream_take.next().await {
                        let mut exec_set = self.exec_set.lock().await;
                        let tick_duration = self.tick_duration.clone();
                        let stats_tx = self.stats_tx.clone();
                        let reports_pool = self.reports_pool.clone();
                        let solver_params = self.solver_params.clone();
                        let laminated_proxy_address = self.laminated_proxy_address;
                        exec_set.spawn(async move {
                            let clean_app_scheduler_solver = CleanAppSchedulerSolver::new(
                                call_pushed.clone(),
                                solver_params,
                                laminated_proxy_address,
                                reports_pool,
                            );
                            if let Ok(clean_app_scheduler_solver) = clean_app_scheduler_solver {
                                let executor =
                                    TimerRequestExecutor::<CleanAppSchedulerSolver<M>>::new(
                                        clean_app_scheduler_solver,
                                        tick_duration,
                                        stats_tx,
                                    );
                                executor.execute(call_pushed).await;
                            } else {
                                println!("Error creating solver: Unknown selector");
                            }
                        });
                    }
                }
                Err(err) => {
                    fatal!("Error reading events from stream: {}", err);
                }
            }
        }
    }
}
