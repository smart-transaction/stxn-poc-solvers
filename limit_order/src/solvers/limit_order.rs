use crate::{
    contracts_abi::{
        call_breaker::{CallBreaker, CallObject, ReturnObject},
        ierc20::{ApproveCall, IERC20Calls},
        laminated_proxy::{LaminatedProxyCalls, PullCall},
        ProxyPushedFilter,
    },
    solver::{self, Solver, SolverError, SolverParams, SolverResponse},
};
use ethers::{
    abi::{self, AbiEncode, Token},
    core::abi::ethabi::ethereum_types::FromDecStrErr,
    prelude::abigen,
    providers::Middleware,
    types::{Address, Bytes, H160, U256}, utils::parse_units,
};
use fixed_hash::rustc_hex::FromHexError;
use parse_duration;
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::sync::Mutex;

abigen!(
    FlashLoan,
    "./abi_town/MockFlashLoan.sol/MockFlashLoan.json";

    SwapPool,
    "./abi_town/MockDaiWethPool.sol/MockDaiWethPool.json";
);

pub const APP_SELECTOR: &str = "FLASHLIQUIDITY.LIMITORDER";
pub const FLASH_LOAN_NAME: &str = "FLASH_LOAN";
pub const SWAP_POOL_NAME: &str = "SWAP_POOL";

pub struct LimitOrderSolver<M> {
    // Solver address
    _solver_address: Address, // To be used after fixing associated data

    // Contract addresses to be called.
    proxy_address: Address,
    call_breaker_address: Address,
    flash_loan_address: Address,
    swap_pool_address: Address,

    // Sequence number for laminator proxy call
    sequence_number: U256,

    // Contracts that are to be called.
    call_breaker_contract: CallBreaker<M>,
    swap_pool_contract: SwapPool<M>,

    // Limit order params
    pub give_token: Result<Address, FromHexError>,
    pub take_token: Result<Address, FromHexError>,
    amount: Result<U256, FromDecStrErr>,
    buy_price: Result<U256, FromDecStrErr>,
    slippage: Result<U256, FromDecStrErr>,
    time_limit: Result<Duration, parse_duration::parse::Error>,

    // Transaction guard
    guard: Arc<Mutex<bool>>,
}

// A clone of the FlashLoanData onchain structure.
// Cannot be imported by abigen due to visibility restriction.
// Should be synchronized with the definition in https://github.com/smart-transaction/stxn-contracts-core/blob/6dc025f53af60a0026aa6a4bb0f1d98a881d978a/src/CallBreakerTypes.sol
struct FlashLoanData {
    provider: Address,
    amount_a: U256,
    amount_b: U256,
}

impl AbiEncode for FlashLoanData {
    fn encode(self) -> Vec<u8> {
        let mut res = self.provider.encode();
        res.extend(self.amount_a.encode());
        res.extend(self.amount_b.encode());
        res
    }
}

impl<M: Middleware + Clone> LimitOrderSolver<M> {
    pub fn new(
        event: ProxyPushedFilter,
        params: SolverParams<M>,
    ) -> Result<LimitOrderSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let flash_liquidity_selector = solver::selector(APP_SELECTOR.to_string());
        if flash_liquidity_selector != event.selector.into() {
            return Err(SolverError::MisleadingSelector(event.selector.into()));
        }

        let flash_loan_address = params.extra_contract_addresses.get(FLASH_LOAN_NAME);
        if let None = flash_loan_address {
            return Err(SolverError::ParamError(
                "missing address for contract FLASH_LOAN".to_string(),
            ));
        }
        let swap_pool_address = params.extra_contract_addresses.get(SWAP_POOL_NAME);
        if let None = swap_pool_address {
            return Err(SolverError::ParamError(
                "missing adsdress for contract SWAP_POOL".to_string(),
            ));
        }
        let mut ret = LimitOrderSolver {
            proxy_address: event.proxy_address,
            call_breaker_address: params.call_breaker_address,
            _solver_address: params.solver_address,
            flash_loan_address: *flash_loan_address.unwrap(),
            swap_pool_address: *swap_pool_address.unwrap(),
            call_breaker_contract: CallBreaker::new(
                params.call_breaker_address,
                params.middleware.clone(),
            ),
            swap_pool_contract: SwapPool::new(
                *swap_pool_address.unwrap(),
                params.middleware.clone(),
            ),
            sequence_number: event.sequence_number,
            give_token: Result::Err(FromHexError::InvalidHexLength),
            take_token: Result::Err(FromHexError::InvalidHexLength),
            amount: Result::Err(FromDecStrErr::InvalidLength),
            buy_price: Result::Err(FromDecStrErr::InvalidLength),
            slippage: Result::Err(FromDecStrErr::InvalidLength),
            time_limit: Result::Err(parse_duration::parse::Error::NoValueFound(
                "Uninitialized value".to_string(),
            )),
            guard: params.guard.clone(),
        };
        // Extract parameters.
        for ad in &event.data_values {
            match ad.name.as_str() {
                "give_token" => ret.give_token = H160::from_str(ad.value.as_str()),
                "take_token" => ret.take_token = H160::from_str(ad.value.as_str()),
                "amount" => ret.amount = U256::from_dec_str(ad.value.as_str()),
                "buy_price" => ret.buy_price = U256::from_dec_str(ad.value.as_str()),
                "slippage" => ret.slippage = U256::from_dec_str(ad.value.as_str()),
                "time_limit" => ret.time_limit = parse_duration::parse(ad.value.as_str()),
                &_ => {}
            }
        }
        // Check that all parameters are successfully extracted.
        if let Err(err) = ret.give_token {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter give_token: {}",
                err
            )));
        }
        if let Err(err) = ret.take_token {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter take_token: {}",
                err
            )));
        }
        if let Err(err) = ret.amount {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter amount: {}",
                err
            )));
        }
        if let Err(err) = ret.buy_price {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter buy_price: {}",
                err
            )));
        }
        if let Err(err) = ret.slippage {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter slippage: {}",
                err
            )));
        }
        if let Err(err) = ret.time_limit {
            return Err(SolverError::ParamError(format!(
                "Error in the parameter time_limit: {}",
                err
            )));
        }
        Ok(ret)
    }
}

impl<M: Middleware> Solver for LimitOrderSolver<M> {
    fn app(&self) -> String {
        return APP_SELECTOR.to_string();
    }

    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error> {
        self.time_limit.clone()
    }

    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError> {
        if let Err(err) = &self.amount {
            return Err(SolverError::ExecError(err.to_string()));
        }
        if let Err(err) = &self.buy_price {
            return Err(SolverError::ExecError(err.to_string()));
        }
        // Check the price
        match self.swap_pool_contract.get_price_of_weth().call().await {
            Ok(current_price) => {
                let desired_price = *self.buy_price.as_ref().ok().unwrap();
                if current_price > desired_price {
                    return Ok(SolverResponse {
                        succeeded: false,
                        message: format!(
                            "The current price {} is higher than the desired {}",
                            current_price, desired_price
                        ),
                    });
                }
            }
            Err(err) => {
                return Err(SolverError::ExecError(err.to_string()));
            }
        }
        Ok(SolverResponse {
            succeeded: true,
            message: "Price conditions are met".to_string(),
        })
    }

    async fn final_exec(&self) -> Result<SolverResponse, SolverError> {
        let hardcoded_weth_liquidity = 100;
        let hardcoded_dai_liquidity = 1000;
        let dai_liquidity_wei = parse_units(hardcoded_dai_liquidity, "ether").ok().unwrap();
        let weth_liquidity_wei = parse_units(hardcoded_weth_liquidity, "ether").ok().unwrap();
        let call_objects = vec![
            CallObject {
                amount: 0.into(),
                addr: self.give_token.ok().unwrap(),
                gas: 10000000.into(),
                callvalue: IERC20Calls::Approve(ApproveCall {
                    spender: self.swap_pool_address,
                    amount: dai_liquidity_wei.into(),
                })
                .encode()
                .into(),
            },
            CallObject {
                amount: 0.into(),
                addr: self.take_token.ok().unwrap(),
                gas: 10000000.into(),
                callvalue: IERC20Calls::Approve(ApproveCall {
                    spender: self.swap_pool_address,
                    amount: weth_liquidity_wei.into(),
                })
                .encode()
                .into(),
            },
            CallObject {
                amount: 0.into(),
                addr: self.swap_pool_address,
                gas: 10000000.into(),
                callvalue: SwapPoolCalls::ProvideLiquidityToDAIETHPool(
                    ProvideLiquidityToDAIETHPoolCall {
                        provider: self.call_breaker_address,
                        amount_0_in: hardcoded_dai_liquidity.into(),
                        amount_1_in: hardcoded_weth_liquidity.into(),
                    },
                )
                .encode()
                .into(),
            },
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
                addr: self.swap_pool_address,
                gas: 10000000.into(),
                callvalue: SwapPoolCalls::CheckSlippage(CheckSlippageCall {
                    max_deviation_percentage: *self.slippage.as_ref().ok().unwrap(),
                })
                .encode()
                .into(),
            },
            CallObject {
                amount: 0.into(),
                addr: self.swap_pool_address,
                gas: 10000000.into(),
                callvalue: SwapPoolCalls::WithdrawLiquidityFromDAIETHPool(
                    WithdrawLiquidityFromDAIETHPoolCall {
                        provider: self.call_breaker_address,
                        amount_0_out: hardcoded_dai_liquidity.into(),
                        amount_1_out: hardcoded_weth_liquidity.into(),
                    },
                )
                .encode()
                .into(),
            },
        ];
        let return_objects_from_pull = vec![
            ReturnObject {
                returnvalue: true.encode().into(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
        ];
        let return_objects = vec![
            ReturnObject {
                returnvalue: true.encode().into(),
            },
            ReturnObject {
                returnvalue: true.encode().into(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
            ReturnObject {
                returnvalue: abi::encode(&[Token::Bytes(return_objects_from_pull.encode())]).into(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
        ];

        let associated_data: Bytes = Bytes::from_str("0x000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000240364975c732e2b61ede80abbc6666bc882f0e45406caaa44bed3e13479c1863632ec94a0831e53d3569cd147364f65fbf6465a359bba763dcbf3dbb7d995bcc0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000014c0aa0ed2e2772d2da76a87403dfa3acfb227f84c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000b").unwrap();
        let hintdices: Bytes = Bytes::from_str("0x00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000581429a2b27ff563be7497b04741d00d647c2b5ea5602bb808ab3284016254c24287482b2054c1c12a544b3241459080c339bcdde156b6f7df323cffa85c8f6d1a8df3b54015c8b7dd463d722f1853a37b8141d463ce84b7e34f2ecb0fafb8463fdf04b0dea4e65b2f6bdaeb618c47b3b6362ddd1cf39970807d3932dabcd052195fb8e8f717e641cfd46bb481f645126212dc3474ef008492d6ccca78636b77a000000000000000000000000000000000000000000000000000000000000000500000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000004").unwrap();
        let flash_loan_data: Bytes = FlashLoanData {
            provider: self.flash_loan_address,
            amount_a: dai_liquidity_wei.into(),
            amount_b: weth_liquidity_wei.into(),
        }
        .encode()
        .into();

        let call_bytes: Bytes = call_objects.encode().into();
        let return_bytes: Bytes = return_objects.encode().into();
        {
            let _guard = self.guard.lock().await;
            match self
                .call_breaker_contract
                .execute_and_verify_with_flashloan(
                    call_bytes,
                    return_bytes,
                    associated_data,
                    hintdices,
                    flash_loan_data,
                )
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
