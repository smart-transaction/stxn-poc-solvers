use crate::contracts_abi::{
    call_breaker::{CallBreaker, CallObject, ReturnObject},
    ierc20::{ApproveCall, IERC20Calls},
    laminated_proxy::{LaminatedProxyCalls, PullCall},
    laminator::ProxyPushedFilter,
};
use ethers::{
    abi::AbiEncode,
    core::abi::ethabi::ethereum_types::FromDecStrErr,
    prelude::abigen,
    providers::Middleware,
    types::{Address, Bytes, H160, H256, U256},
};
use ethers_core::{
    abi::{self, Token},
    utils::parse_units,
};
use fixed_hash::rustc_hex::FromHexError;
use keccak_hash::keccak;
use parse_duration;
use std::{
    collections::HashMap,
    fmt::{self, Display},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

abigen!(
    FlashLoan,
    "./abi_town/MockFlashLoan.sol/MockFlashLoan.json";

    SwapPool,
    "./abi_town/MockDaiWethPool.sol/MockDaiWethPool.json";
);

const APP_SELECTOR: &str = "FLASHLIQUIDITY.LIMITORDER";
const FLASH_LOAN_NAME: &str = "FLASH_LOAN";
const SWAP_POOL_NAME: &str = "SWAP_POOL";

pub enum SolverError {
    UnknownSelector(H256),
    ParamError(String),
    ExecError(String),
}

impl Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverError::UnknownSelector(s) => {
                write!(f, "UnknownSelector: {}", s)
            }
            SolverError::ParamError(s) => {
                write!(f, "Parameter error, \"{}\"", s)
            }
            SolverError::ExecError(s) => {
                write!(f, "Execution error, {}", s)
            }
        }
    }
}

pub struct LimitOrderSolver<M> {
    // Solver address
    solver_address: Address,

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
        return abi::encode(&[
            Token::Bytes(self.provider.encode()),
            Token::Bytes(self.amount_a.encode()),
            Token::Bytes(self.amount_b.encode()),
        ]);
    }
}

impl<M: Middleware> LimitOrderSolver<M> {
    pub fn new(
        event: &ProxyPushedFilter,
        call_breaker_address: Address,
        solver_address: Address,
        extra_contract_addresses: &HashMap<String, Address>,
        middleware: Arc<M>,
    ) -> Result<LimitOrderSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let flash_liquidity_selector = Self::selector();
        if flash_liquidity_selector != event.selector.into() {
            return Err(SolverError::UnknownSelector(event.selector.into()));
        }

        let flash_loan_address = extra_contract_addresses.get(FLASH_LOAN_NAME);
        if let None = flash_loan_address {
            return Err(SolverError::ParamError(
                "missing address for contract FLASH_LOAN".to_string(),
            ));
        }
        let swap_pool_address = extra_contract_addresses.get(SWAP_POOL_NAME);
        if let None = swap_pool_address {
            return Err(SolverError::ParamError(
                "missing adsdress for contract SWAP_POOL".to_string(),
            ));
        }
        let mut ret = LimitOrderSolver {
            proxy_address: event.proxy_address,
            call_breaker_address,
            solver_address,
            flash_loan_address: *flash_loan_address.unwrap(),
            swap_pool_address: *swap_pool_address.unwrap(),
            call_breaker_contract: CallBreaker::new(call_breaker_address, middleware.clone()),
            swap_pool_contract: SwapPool::new(*swap_pool_address.unwrap(), middleware.clone()),
            sequence_number: event.sequence_number,
            give_token: Result::Err(FromHexError::InvalidHexLength),
            take_token: Result::Err(FromHexError::InvalidHexLength),
            amount: Result::Err(FromDecStrErr::InvalidLength),
            buy_price: Result::Err(FromDecStrErr::InvalidLength),
            slippage: Result::Err(FromDecStrErr::InvalidLength),
            time_limit: Result::Err(parse_duration::parse::Error::NoValueFound(
                "Uninitialized value".to_string(),
            )),
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

    pub fn selector() -> H256 {
        keccak(APP_SELECTOR.encode()).as_fixed_bytes().into()
    }
}

impl<M: Middleware> LimitOrderSolver<M> {
    pub fn app(&self) -> String {
        return APP_SELECTOR.to_string();
    }
    pub fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error> {
        self.time_limit.clone()
    }
    pub async fn exec_solver_step(&self) -> Result<bool, SolverError> {
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
                    println!(
                        "The current price {} is higher than the desired {}",
                        current_price, desired_price
                    );
                    return Ok(false);
                }
            }
            Err(err) => {
                return Err(SolverError::ExecError(err.to_string()));
            }
        }
        Ok(true)
    }

    pub async fn final_exec(&self) -> Result<bool, SolverError> {
        let hardcoded_weth_liquidity = 100;
        let hardcoded_dai_liquidity = 1000;
        let dai_liquidity_wei = parse_units(hardcoded_dai_liquidity, "ether").ok().unwrap();
        let weth_liquidity_wei = parse_units(hardcoded_weth_liquidity, "ether").ok().unwrap();
        let call_objects = vec![
            CallObject {
                amount: 0.into(),
                addr: self.give_token.ok().unwrap(),
                gas: 1000000.into(),
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
                gas: 1000000.into(),
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
                gas: 1000000.into(),
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
                gas: 1000000.into(),
                callvalue: LaminatedProxyCalls::Pull(PullCall {
                    seq_number: self.sequence_number,
                })
                .encode()
                .into(),
            },
            CallObject {
                amount: 0.into(),
                addr: self.swap_pool_address,
                gas: 1000000.into(),
                callvalue: SwapPoolCalls::CheckSlippage(CheckSlippageCall {
                    max_deviation_percentage: *self.slippage.as_ref().ok().unwrap(),
                })
                .encode()
                .into(),
            },
            CallObject {
                amount: 0.into(),
                addr: self.swap_pool_address,
                gas: 1000000.into(),
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
                returnvalue: Bytes::new(),
            },
            ReturnObject {
                returnvalue: true.encode().into(),
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
                returnvalue: return_objects_from_pull.encode().into(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
            ReturnObject {
                returnvalue: Bytes::new(),
            },
        ];
        let associated_data_keys: Vec<H256> = vec![
            keccak("tipYourBartender".encode()).as_fixed_bytes().into(),
            keccak("pullIndex".encode()).as_fixed_bytes().into(),
        ];
        let associated_data_values =
            vec![self.solver_address.encode(), self.sequence_number.encode()];
        let associated_data: Bytes = abi::encode(&[
            Token::Bytes(associated_data_keys.encode()),
            Token::Bytes(associated_data_values.encode()),
        ])
        .into();
        let hintdices_keys: Vec<H256> = vec![
            keccak(call_objects[0].clone().encode())
                .as_fixed_bytes()
                .into(),
            keccak(call_objects[1].clone().encode())
                .as_fixed_bytes()
                .into(),
            keccak(call_objects[2].clone().encode())
                .as_fixed_bytes()
                .into(),
            keccak(call_objects[3].clone().encode())
                .as_fixed_bytes()
                .into(),
            keccak(call_objects[4].clone().encode())
                .as_fixed_bytes()
                .into(),
            keccak(call_objects[5].clone().encode())
                .as_fixed_bytes()
                .into(),
        ];
        let hintdices_values: Vec<U256> =
            vec![0.into(), 1.into(), 2.into(), 3.into(), 4.into(), 5.into()];
        let hintdices: Bytes = abi::encode(&[
            Token::Bytes(hintdices_keys.encode()),
            Token::Bytes(hintdices_values.encode()),
        ])
        .into();
        let flash_loan_data: Bytes = FlashLoanData {
            provider: self.flash_loan_address,
            amount_a: hardcoded_dai_liquidity.into(),
            amount_b: hardcoded_weth_liquidity.into(),
        }
        .encode()
        .into();

        let call_bytes: Bytes = call_objects.encode().into();
        let return_bytes: Bytes = return_objects.encode().into();
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
                        println!("Receipt: {:#?}", receipt);
                        if let Some(receipt) = receipt {
                            if let Some(status) = receipt.status {
                                return Ok(status != 0.into());
                            }
                        }
                        return Ok(false);
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
    }
}
