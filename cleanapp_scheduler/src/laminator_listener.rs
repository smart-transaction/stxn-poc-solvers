use ethers::{
    abi::Address,
    providers::{Middleware, StreamExt},
    types::{BlockNumber, H256, U256},
};
use fatal::fatal;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc::Sender, Mutex},
    task::JoinSet,
};

use crate::{
    contracts_abi::laminator::{Laminator, ProxyPushedFilter},
    solver::{selector, SolverParams},
    solvers::cleanapp_scheduler::{self, CleanAppSchedulerSolver},
    stats::TimerExecutorStats,
    timer_executor::TimerRequestExecutor,
};

pub struct LaminatorListener<M: Clone> {
    // The address of the laminator contract.
    laminator_address: Address,

    // The middleware to be used
    middleware: Arc<M>,

    // Mapping of app selectors to solver params.
    solvers_params: HashMap<H256, SolverParams<M>>,

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
        laminator_address: Address,
        middleware: Arc<M>,
        solvers_params: HashMap<H256, SolverParams<M>>,
        exec_set: Arc<Mutex<JoinSet<()>>>,
        tick_duration: Duration,
        stats_tx: Sender<TimerExecutorStats>,
        reports_pool: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>>,
    ) -> LaminatorListener<M> {
        LaminatorListener::<M> {
            laminator_address,
            middleware,
            solvers_params,
            exec_set,
            tick_duration,
            stats_tx,
            reports_pool,
        }
    }

    pub async fn listen(&mut self) {
        let laminator_contract = Laminator::new(self.laminator_address, self.middleware.clone());
        let events = laminator_contract
            .event::<ProxyPushedFilter>()
            .from_block(BlockNumber::Latest);
        loop {
            match events.stream().await {
                Ok(stream) => {
                    let mut stream_take = stream.take(10);
                    println!("Listening the event ProxyPushed ...");
                    while let Some(Ok(proxy_pushed)) = stream_take.next().await {
                        if let Some(solver_params) =
                            self.solvers_params.get(&proxy_pushed.selector.into())
                        {
                            let mut exec_set = self.exec_set.lock().await;
                            let solver_params = solver_params.clone();
                            let tick_duration = self.tick_duration.clone();
                            let stats_tx = self.stats_tx.clone();
                            let reports_pool= self.reports_pool.clone();
                            exec_set.spawn(async move {
                                let cleanapp_scheduler_selector =
                                    selector(cleanapp_scheduler::APP_SELECTOR.to_string());
                                let event_selector: H256 = proxy_pushed.selector.into();
                                if event_selector == cleanapp_scheduler_selector {
                                    let clean_app_scheduler_solver = CleanAppSchedulerSolver::new(
                                        proxy_pushed.clone(),
                                        solver_params,
                                        reports_pool,
                                    );
                                    if let Ok(clean_app_scheduler_solver) =
                                        clean_app_scheduler_solver
                                    {
                                        let executor = TimerRequestExecutor::<
                                            CleanAppSchedulerSolver<M>,
                                        >::new(
                                            clean_app_scheduler_solver,
                                            tick_duration,
                                            stats_tx,
                                        );
                                        executor.execute(proxy_pushed).await;
                                    } else {
                                        println!("Error creating solver: Unknown selector");
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
