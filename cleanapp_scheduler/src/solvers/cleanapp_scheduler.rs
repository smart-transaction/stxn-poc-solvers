use crate::{
    contracts_abi::{CallBreaker, ProxyPushedFilter},
    solver::{self, Solver, SolverError, SolverParams, SolverResponse},
};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use cron::{
    error::{Error as CronError, ErrorKind},
    Schedule,
};
use ethers::{
    contract::abigen,
    providers::Middleware,
    types::{Address, U256},
};
use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{sync::Mutex, time::Instant};

abigen!(
  KitnDisbursement,
  "./abi_town/KitnDisbursement.sol/KitnDisbursement.json";
);

pub const APP_SELECTOR: &str = "CLEANAPP.SCHEDULER";
pub const KITN_DISBURSEMENT_NAME: &str = "KITN_DISBURSEMENT";

pub struct CleanAppSchedulerSolver<M> {
    // Solver address
    solver_address: Address,

    // Contract addresses to be called.
    proxy_address: Address,
    call_breaker_address: Address,
    kitn_disbursement_address: Address,

    // Contracts
    kitn_disbursement_contract: KitnDisbursement<M>,
    call_breaker_contract: CallBreaker<M>,

    // Schedule Param
    schedule: Result<Schedule, CronError>,

    // Transaction Guard
    guard: Arc<Mutex<bool>>,

    // Reports Pool
    reports_pool: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>>,
}

impl<M: Middleware + Clone> CleanAppSchedulerSolver<M> {
    pub fn new(
        event: ProxyPushedFilter,
        params: SolverParams<M>,
        reports_pool: Arc<Mutex<HashMap<String, HashMap<Address, U256>>>>,
    ) -> Result<CleanAppSchedulerSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let cleanapp_scheduler_selector = solver::selector(APP_SELECTOR.to_string());
        if cleanapp_scheduler_selector != event.selector.into() {
            return Err(SolverError::MisleadingSelector(event.selector.into()));
        }

        let kitn_disbursement_address = params.extra_contract_addresses.get(KITN_DISBURSEMENT_NAME);
        if let None = kitn_disbursement_address {
            return Err(SolverError::ParamError(
                "missing address for contract TOKEN_DISBURSEMENT".to_string(),
            ));
        }

        let mut ret = CleanAppSchedulerSolver {
            solver_address: params.solver_address,
            proxy_address: event.proxy_address,
            call_breaker_address: params.call_breaker_address,
            kitn_disbursement_address: *kitn_disbursement_address.unwrap(),
            call_breaker_contract: CallBreaker::new(
                params.call_breaker_address,
                params.middleware.clone(),
            ),
            kitn_disbursement_contract: KitnDisbursement::new(
                *kitn_disbursement_address.unwrap(),
                params.middleware.clone(),
            ),
            schedule: Result::Err(CronError::from(ErrorKind::Expression(
                "Uninitialized".to_string(),
            ))),
            guard: params.guard,
            reports_pool,
        };

        // Extract parameters.
        for ad in &event.data_values {
            match ad.name.as_str() {
                "schedule" => ret.schedule = Schedule::from_str(ad.value.as_str()),
                &_ => {}
            }
        }
        // Check that all parameters are successfully extracted.
        if let Err(err) = ret.schedule {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter give_token: {}",
                err
            )));
        }

        Ok(ret)
    }
}

impl<M: Middleware> Solver for CleanAppSchedulerSolver<M> {
    fn app(&self) -> String {
        APP_SELECTOR.to_string()
    }

    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error> {
        // Return max. duration 24 hours, considers the schedule
        Ok(Duration::new(60 * 60 * 24, 0))
    }

    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError> {
        // Check if the schedule is triggered.
        match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(now) => {
                let now =
                    DateTime::from_timestamp(i64::from_ne_bytes(now.as_secs().to_ne_bytes()), 0)
                        .unwrap();
                for trigger_time in self.schedule.as_ref().unwrap().upcoming(Utc).take(1) {
                    if trigger_time <= now {
                        return Ok(SolverResponse {
                            succeeded: true,
                            message: format!("Triggered at {}", now),
                        });
                    }
                }
            }
            Err(err) => {
                return Err(SolverError::ExecError(format!(
                    "Solver execution error: {}",
                    err
                )));
            }
        }

        // Return false to show that the condition han't been met.
        Ok(SolverResponse {
            succeeded: false,
            message: "Not triggered yet".to_string(),
        })
    }

    async fn final_exec(&self) -> Result<SolverResponse, SolverError> {
        
        Ok(SolverResponse {
            succeeded: true,
            message: "Executed successfully".to_string(),
        })
    }
}
