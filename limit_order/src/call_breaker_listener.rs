use ethers::{
    abi::Address,
    providers::{Middleware, StreamExt},
    types::{BlockNumber, H256},
};
use fatal::fatal;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc::Sender, Mutex},
    task::JoinSet,
};

use crate::{
    contracts_abi::call_breaker::{CallBreaker, UserObjectivePushedFilter},
    solver::{selector, SolverParams},
    solvers::limit_order::LimitOrderSolver,
    stats::TimerExecutorStats,
    timer_executor::TimerRequestExecutor,
};

pub struct CallBreakerListener<M: Clone> {
    // The address of the call breaker contract.
    call_breaker_address: Address,

    // The middleware to be used
    middleware: Arc<M>,

    // Mapping of app IDs to solver params.
    solvers_params: HashMap<H256, SolverParams<M>>,

    // JoinSet for using for executors spawning.
    exec_set: Arc<Mutex<JoinSet<()>>>,

    // Execution tick duration
    tick_duration: Duration,

    // The channel for sending current stats
    stats_tx: Sender<TimerExecutorStats>,
}

impl<M: Middleware + Clone + 'static> CallBreakerListener<M> {
    pub fn new(
        call_breaker_address: Address,
        middleware: Arc<M>,
        solvers_params: HashMap<H256, SolverParams<M>>,
        exec_set: Arc<Mutex<JoinSet<()>>>,
        tick_duration: Duration,
        stats_tx: Sender<TimerExecutorStats>,
    ) -> CallBreakerListener<M> {
        CallBreakerListener::<M> {
            call_breaker_address,
            middleware,
            solvers_params,
            exec_set,
            tick_duration,
            stats_tx,
        }
    }

    pub async fn listen(&mut self) {
        let call_breaker_contract =
            CallBreaker::new(self.call_breaker_address, self.middleware.clone());
        let events = call_breaker_contract
            .event::<UserObjectivePushedFilter>()
            .from_block(BlockNumber::Latest);
        loop {
            match events.stream().await {
                Ok(stream) => {
                    let mut stream_take = stream.take(10);
                    println!("Listening the event UserObjectivePushed ...");
                    while let Some(Ok(user_objective_pushed)) = stream_take.next().await {
                        let app_id: H256 = user_objective_pushed.app_id;

                        if let Some(solver_params) = self.solvers_params.get(&app_id) {
                            let mut exec_set = self.exec_set.lock().await;
                            let solver_params = solver_params.clone();
                            let tick_duration = self.tick_duration.clone();
                            let stats_tx = self.stats_tx.clone();
                            exec_set.spawn(async move {
                                let limit_order_app_id =
                                    selector("FLASHLIQUIDITY.LIMITORDER".to_string());
                                let event_app_id: H256 = user_objective_pushed.app_id;
                                if event_app_id == limit_order_app_id {
                                    let limit_order_solver = LimitOrderSolver::new(
                                        user_objective_pushed.clone(),
                                        solver_params.clone(),
                                    );
                                    if let Ok(limit_order_solver) = limit_order_solver {
                                        let executor =
                                            TimerRequestExecutor::<LimitOrderSolver<M>>::new(
                                                limit_order_solver,
                                                tick_duration,
                                                stats_tx,
                                            );
                                        executor.execute(user_objective_pushed).await;
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
