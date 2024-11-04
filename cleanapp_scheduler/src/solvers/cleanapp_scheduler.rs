use crate::{
    contracts_abi::{
        AdditionalData, CallBreaker, CallObject, CallPushedFilter, LaminatedProxyCalls, PullCall,
        ReturnObject,
    },
    solver::{Solver, SolverError, SolverParams, SolverResponse},
};
use chrono::{DateTime, Utc};
use cron::Schedule;
use ethers::{
    abi::{self, AbiEncode, Token},
    contract::abigen,
    providers::Middleware,
    types::{Address, Bytes, U256},
};
use keccak_hash::keccak;
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

    // Solver address
    solver_address: Address,

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

    // Transaction Guard
    guard: Arc<Mutex<bool>>,

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
    ) -> Result<CleanAppSchedulerSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let mut ret = CleanAppSchedulerSolver {
            sequence_number: event.sequence_number,
            solver_address: params.solver_address,
            proxy_address,
            kitn_disbursement_scheduler_address,
            call_breaker_contract: CallBreaker::new(
                params.call_breaker_address,
                params.middleware.clone(),
            ),
            schedule_string: String::new(),
            trigger_time: Err(SolverError::ParamError(
                "Missing CRON parameter".to_string(),
            )),
            guard: params.guard,
            reports_pool,
        };

        // Extract parameters.
        let mut schedule_extracted = false;
        for ad in &event.data {
            match ad.name.as_str() {
                "CRON" => {
                    ret.schedule_string = ad.value.clone();
                    match Schedule::from_str(ad.value.as_str()) {
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
                }
                &_ => {}
            }
        }
        // Check that all parameters are successfully extracted.
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
        const MAX_BATCH_SIZE: usize = 100;
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

        for (account, amount) in self.reports_pool.lock().await.iter() {
            receivers.push(*account);
            amounts.push(*amount);
        }

        let disbursal_data: Bytes = DisbursalData { receivers, amounts }.encode().into();

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
                gas: 10000000.into(),
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

        let associated_data: Bytes = vec![
            AdditionalData {
                key: keccak("tipYourBartender".encode()).into(),
                value: self.solver_address.encode().into(),
            },
            AdditionalData {
                key: keccak("pullIndex".encode()).into(),
                value: self.sequence_number.encode().into(),
            },
            AdditionalData {
                key: keccak("KITNDisbursalData".encode()).into(),
                value: disbursal_data,
            },
            AdditionalData {
                key: keccak("CleanAppSignature".encode()).into(),
                value: "rsv".encode().into(),
            }
        ]
        .encode()
        .into();

        let call_obj_index_0: U256 = 0.into();
        let call_obj_index_1: U256 = 0.into();
        let hintindices: Bytes = vec![
            AdditionalData {
                key: keccak(call_objects[0].clone().encode()).into(),
                value: call_obj_index_0.encode().into(),
            },
            AdditionalData {
                key: keccak(call_objects[0].clone().encode()).into(),
                value: call_obj_index_1.encode().into(),
            },
        ]
        .encode()
        .into();

        let call_bytes: Bytes = call_objects.encode().into();
        let return_bytes: Bytes = return_objects.encode().into();
        {
            let _guard = self.guard.lock().await;
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
                                        let mut reports = self.reports_pool.lock().await;
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
