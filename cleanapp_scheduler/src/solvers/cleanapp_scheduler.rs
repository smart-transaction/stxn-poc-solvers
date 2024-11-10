use crate::{
    contracts_abi::{
        CallBreaker, CallObject, CallPushedFilter, LaminatedProxyCalls, PullCall,
        ReturnObject,
    }, encoded_data::{get_associated_data, get_disbursed_data}, solver::{Solver, SolverError, SolverParams, SolverResponse}
};
use chrono::{DateTime, Utc};
use cron::Schedule;
use ethers::{
    abi::{self, AbiEncode, Token},
    contract::abigen,
    providers::Middleware,
    types::{Address, Bytes, U256},
};
use std::{collections::HashMap, str::FromStr, sync::Arc, time::SystemTime};
use tokio::sync::Mutex;

abigen!(
  KITNDisburmentScheduler,
  "./abi_town/KITNDisburmentScheduler.sol/KITNDisburmentScheduler.json",
  derives(serde::Deserialize, serde::Serialize);
);

pub const APP_SELECTOR: &str = "CLEANAPP.SCHEDULER";

pub struct CleanAppSchedulerSolver<M> {
    // Sequence number for laminator proxy call
    sequence_number: U256,

    // Proxy Address
    proxy_address: Address,

    // KITN Disbursement Address
    kitn_disbursement_scheduler_address: Address,

    // Contracts
    call_breaker_contract: CallBreaker<M>,

    // Schedule String
    schedule_string: String,

    // Trigger time
    trigger_time: Result<DateTime<Utc>, SolverError>,

    // Reports Pool
    reports_pool: Arc<Mutex<HashMap<Address, U256>>>,
}

impl<M: Middleware + Clone> CleanAppSchedulerSolver<M> {
    pub fn new(
        event: CallPushedFilter,
        params: SolverParams<M>,
        proxy_address: Address,
        kitn_disbursement_scheduler_address: Address,
        reports_pool: Arc<Mutex<HashMap<Address, U256>>>,
        cron: String,
    ) -> Result<CleanAppSchedulerSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let mut ret = CleanAppSchedulerSolver {
            sequence_number: event.sequence_number,
            proxy_address,
            kitn_disbursement_scheduler_address,
            call_breaker_contract: CallBreaker::new(
                params.call_breaker_address,
                params.middleware.clone(),
            ),
            schedule_string: cron,
            trigger_time: Err(SolverError::ParamError(
                "Missing CRON parameter".to_string(),
            )),
            reports_pool,
        };

        let mut schedule_extracted = false;
        // Check that all parameters are successfully extracted.
        match Schedule::from_str(ret.schedule_string.as_str()) {
            Ok(schedule) => {
                for trigger_time in schedule.upcoming(Utc).take(1) {
                    ret.trigger_time = Ok(trigger_time);
                }
                schedule_extracted = true;
            }
            Err(err) => {
                ret.trigger_time = Err(SolverError::ParamError(format!(
                    "Error parsing CRON parameter: {}",
                    err
                )));
            }
        }
        if !schedule_extracted {
            return Err(SolverError::ParamError(
                "Missing schedule, the solver won't run".to_string(),
            ));
        }

        Ok(ret)
    }
}

impl<M: Middleware> Solver for CleanAppSchedulerSolver<M> {
    fn app(&self) -> String {
        APP_SELECTOR.to_string()
    }

    fn schedule_time(&self) -> Result<DateTime<Utc>, SolverError> {
        self.trigger_time.clone()
    }

    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError> {
        if let Err(err) = self.trigger_time.clone() {
            return Err(err);
        }
        let trigger_time = self.trigger_time.clone().unwrap();
        // Check if the schedule is triggered.
        const MAX_BATCH_SIZE: usize = 10;
        match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(now) => {
                let now =
                    DateTime::from_timestamp(i64::from_ne_bytes(now.as_secs().to_ne_bytes()), 0)
                        .unwrap();
                if trigger_time <= now {
                    let reports = self.reports_pool.lock().await;
                    if !reports.is_empty() {
                        return Ok(SolverResponse {
                            succeeded: true,
                            message: format!("Triggered at {}", now),
                            remaining_secs: 0,
                        });
                    } else {
                        return Ok(SolverResponse {
                            succeeded: false,
                            message: "Not triggered, the pool is empty".to_string(),
                            remaining_secs: 0,
                        });
                    }
                } else {
                    let reports = self.reports_pool.lock().await;
                    if reports.len() >= MAX_BATCH_SIZE {
                        return Ok(SolverResponse {
                            succeeded: true,
                            message: format!("Triggered at {} as the batch is complete", now),
                            remaining_secs: 0,
                        });
                    } else {
                        return Ok(SolverResponse {
                            succeeded: false,
                            message: "Not triggered yet, the schedule time wasn't reached yet"
                                .to_string(),
                            remaining_secs: (trigger_time - now).num_seconds(),
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
    }

    async fn final_exec(&self) -> Result<SolverResponse, SolverError> {
        let mut receivers: Vec<Address> = Vec::new();
        let mut amounts: Vec<U256> = Vec::new();

        let mut reports = self.reports_pool.lock().await;
        for (account, amount) in reports.iter() {
            receivers.push(*account);
            amounts.push(*amount);
        }

        let disbursal_data = get_disbursed_data(receivers.clone(), amounts.clone());

        let call_objects = vec![
            CallObject {
                amount: 0.into(),
                addr: self.proxy_address,
                gas: 10000000.into(),
                callvalue: LaminatedProxyCalls::Pull(PullCall {
                    seq_number: self.sequence_number,
                })
                .encode()
                .into(),
            },
            CallObject {
                amount: 0.into(),
                addr: self.kitn_disbursement_scheduler_address,
                gas: 1000000.into(),
                callvalue: KITNDisburmentSchedulerCalls::VerifySignature(VerifySignatureCall {
                    data: disbursal_data.clone(),
                })
                .encode()
                .into(),
            },
        ];
        let next_sequence_number = self.sequence_number + 1;
        let return_objects_from_pull = vec![
            ReturnObject {
                returnvalue: Bytes::new(),
            },
            ReturnObject {
                returnvalue: next_sequence_number.encode().into(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
        ];
        let return_objects = vec![
            ReturnObject {
                returnvalue: abi::encode(&[Token::Bytes(return_objects_from_pull.encode())]).into(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
        ];

        let associated_data = get_associated_data(self.sequence_number, receivers, amounts);
        let hintindices = Bytes::from_str("0x00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000c0baed237ba5681f7a9e0892d5d807f7bddae6ccb06e0a053b4b358cad56dfc2b1000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000b09eb645b7de126aeb2d91436e34148ebde4ff228768eb684ecb19bd1524ac06000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000001").unwrap();

        let call_bytes: Bytes = call_objects.encode().into();
        let return_bytes: Bytes = return_objects.encode().into();
        {
            match self
                .call_breaker_contract
                .execute_and_verify(call_bytes, return_bytes, associated_data, hintindices)
                .gas(10000000)
                .send()
                .await
            {
                Ok(pending) => {
                    println!("Transaction is sent, txhash: {}", pending.tx_hash());
                    match pending.await {
                        Ok(receipt) => {
                            if let Some(receipt) = receipt {
                                if let Some(status) = receipt.status {
                                    if status > 0.into() {
                                        reports.clear();
                                    }
                                    return Ok(SolverResponse {
                                        succeeded: status != 0.into(),
                                        message: format!("Transaction status: {}", status),
                                        remaining_secs: 0,
                                    });
                                }
                            }
                            return Ok(SolverResponse {
                                succeeded: false,
                                message: "transaction status wasn't received".to_string(),
                                remaining_secs: 0,
                            });
                        }
                        Err(err) => {
                            return Err(SolverError::ExecError(format!(
                                "Final execution error: {}",
                                err
                            )));
                        }
                    }
                }
                Err(err) => {
                    return Err(SolverError::ExecError(format!(
                        "Final execution error: {}",
                        err
                    )));
                }
            }
        };
    }
}
