use ethers::{
    prelude::abigen, providers::Middleware, types::{Address, H256, U256},
    core::abi::ethabi::ethereum_types::FromDecStrErr,
};
use ethers_core::abi;
use ethers_core::abi::Token;
use keccak_hash::keccak;
use parse_duration;
use std::{collections::HashMap, fmt::{self, Display}, sync::Arc, time::Duration};

use crate::contracts_abi::laminator::AdditionalData;

abigen!(
    FlashLoan,
    "./abi_town/MockFlashLoan.sol/MockFlashLoan.json";

    SwapPool,
    "./abi_town/MockDaiWethPool.sol/MockDaiWethPool.json";
);

const APP_SELECTOR: &str = "LIMIT_ORDER";
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
    flash_loan_contract: FlashLoan<M>,
    swap_pool_contract: SwapPool<M>,
    amount: Result<U256, FromDecStrErr>,
    price: Result<U256, FromDecStrErr>,
    slippage: Result<U256, FromDecStrErr>,
    time_limit: Result<Duration, parse_duration::parse::Error>,
}

impl<M: Middleware> LimitOrderSolver<M> {
    pub fn new(selector: H256, contract_addresses: &HashMap<String, Address>, middleware: Arc<M>, params: &Vec<AdditionalData>) -> Result<LimitOrderSolver<M>, SolverError> {
        let flash_liquidity_selector = Self::selector();
        if selector != flash_liquidity_selector.into() {
            return Err(SolverError::UnknownSelector(selector));
        }
    
        let flash_loan_address = contract_addresses.get(FLASH_LOAN_NAME);
        if let None = flash_loan_address {
            return Err(SolverError::ParamError("missing address for contract FLASH_LOAN".to_string()));
        }
        let swap_pool_address = contract_addresses.get(SWAP_POOL_NAME);
        if let None = swap_pool_address {
            return Err(SolverError::ParamError("missing adsdress for contract SWAP_POOL".to_string()));
        }
        let mut ret = LimitOrderSolver {
            
            flash_loan_contract: FlashLoan::new(*flash_loan_address.unwrap(), middleware.clone()),
            swap_pool_contract: SwapPool::new(*swap_pool_address.unwrap(), middleware.clone()),
            amount: Result::Err(FromDecStrErr::InvalidLength),
            price: Result::Err(FromDecStrErr::InvalidLength),
            slippage: Result::Err(FromDecStrErr::InvalidLength),
            time_limit: Result::Err(parse_duration::parse::Error::NoValueFound(
                "Uninitialized value".to_string(),
            )),
        };
        for ad in params {
            match ad.name.as_str() {
                "amount" => ret.amount = U256::from_dec_str(ad.value.as_str()),
                "price" => ret.price = U256::from_dec_str(ad.value.as_str()),
                "slippage" => ret.slippage = U256::from_dec_str(ad.value.as_str()),
                "time_limit" => ret.time_limit = parse_duration::parse(ad.value.as_str()),
                &_ => {}
            }
        }

        Ok(ret)
    }

    pub fn selector() -> H256 {
        keccak(abi::encode(&[Token::Bytes(Vec::from(
            APP_SELECTOR.as_bytes(),
        ))]))
        .as_fixed_bytes()
        .into()
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
        if let Err(err) = &self.price {
            return Err(SolverError::ExecError(err.to_string()));
        }
        // Check the flash loan
        match self.flash_loan_contract.max_flash_loan().call().await {
            Ok(res) => {
                let weth_balance = res.0;
                let amount_256 = self.amount.as_ref().ok().unwrap();
                if weth_balance < *amount_256 {
                    return Ok(false);
                }
            }
            Err(err) => {
                return Err(SolverError::ExecError(err.to_string()));
            }
        }
        // Check the price
        match self.swap_pool_contract.get_price_of_dai().call().await {
            Ok(res) => {
                let price_256 = self.price.as_ref().ok().unwrap();
                if res > *price_256 {
                    return Ok(false);
                }
            }
            Err(err) => {
                return Err(SolverError::ExecError(err.to_string()));
            }
        }
        Ok(true)
    }
}
