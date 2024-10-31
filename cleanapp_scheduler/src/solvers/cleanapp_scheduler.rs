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
    types::{Address, Bytes, H160, U256}, utils::parse_units,
};
use keccak_hash::keccak;
use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};
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

    // Contracts
    call_breaker_contract: CallBreaker<M>,

    // Schedule String
    schedule_string: String,

    // Trigger time
    trigger_time: DateTime<Utc>,

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
        reports_pool: Arc<Mutex<HashMap<Address, U256>>>,
    ) -> Result<CleanAppSchedulerSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let mut ret = CleanAppSchedulerSolver {
            sequence_number: event.sequence_number,
            solver_address: params.solver_address,
            proxy_address,
            call_breaker_contract: CallBreaker::new(
                params.call_breaker_address,
                params.middleware.clone(),
            ),
            schedule_string: String::new(),
            trigger_time: DateTime::from_timestamp_nanos(0),
            guard: params.guard,
            reports_pool,
        };

        // Extract parameters.
        let mut schedule_extracted = false;
        for ad in &event.data {
            match ad.name.as_str() {
                "CRON" => {
                    ret.schedule_string = ad.value.clone();
                    let schedule = Schedule::from_str(ad.value.as_str());
                    for trigger_time in schedule.as_ref().unwrap().upcoming(Utc).take(1) {
                        ret.trigger_time = trigger_time;
                    }
                    schedule_extracted = true;
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

    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error> {
        // Return max. duration 24 hours, considers the schedule
        Ok(Duration::new(60 * 60 * 24, 0))
    }

    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError> {
        // Check if the schedule is triggered.
        const MAX_BATCH_SIZE: usize = 100;
        match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(now) => {
                let now =
                    DateTime::from_timestamp(i64::from_ne_bytes(now.as_secs().to_ne_bytes()), 0)
                        .unwrap();
                if self.trigger_time <= now {
                    return Ok(SolverResponse {
                        succeeded: true,
                        message: format!("Triggered at {}", now),
                    });
                } else {
                    let reports = self.reports_pool.lock().await;
                    if reports.len() >= MAX_BATCH_SIZE {
                        return Ok(SolverResponse {
                            succeeded: true,
                            message: format!("Triggered at {} as the batch is complete", now),
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

        // Return false to show that the condition hasn't been met.
        Ok(SolverResponse {
            succeeded: false,
            message: "Not triggered yet".to_string(),
        })
    }

    async fn final_exec(&self) -> Result<SolverResponse, SolverError> {
        let call_objects = vec![CallObject {
            amount: 0.into(),
            addr: self.proxy_address,
            gas: 10000000.into(),
            callvalue: LaminatedProxyCalls::Pull(PullCall {
                seq_number: self.sequence_number,
            })
            .encode()
            .into(),
        }];
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
        let return_objects = vec![ReturnObject {
            returnvalue: abi::encode(&[Token::Bytes(return_objects_from_pull.encode())]).into(),
        }];

        let mut receivers: Vec<Address> = Vec::new();
        let mut amounts: Vec<U256> = Vec::new();

        // Temporary example values
        receivers.push(H160::from_str("0xB47eb09d1953Ae9a8c983177b3f78A873edf925d").unwrap());
        let amount: U256 = parse_units(1, "ether").ok().unwrap().into();
        amounts.push(amount);

        // for (k, v) in self.reports_pool.lock().await.iter() {
        //     if *k == self.schedule_string {
        //         for (account, amount) in v.iter() {
        //             receivers.push(*account);
        //             amounts.push(*amount);
        //         }
        //     }
        // }

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
                value: DisbursalData { receivers, amounts }.encode().into(),
            },
        ]
        .encode()
        .into();

        let call_obj_index: U256 = 0.into();
        let hintindices: Bytes = vec![AdditionalData {
            key: keccak(call_objects[0].clone().encode()).into(),
            value: call_obj_index.encode().into(),
        }]
        .encode()
        .into();

        let call_bytes: Bytes = call_objects.encode().into();
        let return_bytes: Bytes = return_objects.encode().into();

        println!("{}", call_bytes);
        println!("{}", return_bytes);
        println!("{}", associated_data);
        println!("{}", hintindices);

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
                                    return Ok(SolverResponse {
                                        succeeded: status != 0.into(),
                                        message: format!("Transaction status: {}", status),
                                    });
                                }
                            }
                            return Ok(SolverResponse {
                                succeeded: false,
                                message: "transaction status wasn't received".to_string(),
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
