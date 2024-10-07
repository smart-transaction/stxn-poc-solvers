use crate::{
    contracts_abi::laminator::AdditionalData,
    solver_factory::{Solver, SolverError},
};
use bigdecimal::{BigDecimal, ParseBigDecimalError};
use ethers::types::H256;
use ethers_core::abi;
use ethers_core::abi::Token;
use keccak_hash::keccak;
use parse_duration;
use std::{str::FromStr, time::Duration};

const APP_SELECTOR: &str = "LIMIT_ORDER";

pub struct LimitOrderSolver {
    pub token: String,
    pub price: Result<BigDecimal, ParseBigDecimalError>,
    pub slippage: Result<BigDecimal, ParseBigDecimalError>,
    pub time_limit: Result<Duration, parse_duration::parse::Error>,
}

impl LimitOrderSolver {
    pub fn new(params: &Vec<AdditionalData>) -> LimitOrderSolver {
        let mut ret = LimitOrderSolver {
            token: String::new(),
            price: Result::Err(ParseBigDecimalError::Empty),
            slippage: Result::Err(ParseBigDecimalError::Empty),
            time_limit: Result::Err(parse_duration::parse::Error::NoValueFound(
                "Uninitialized value".to_string(),
            )),
        };
        for ad in params {
            match ad.name.as_str() {
                "token" => {
                    ret.token = ad.value.clone();
                }
                "price" => ret.price = BigDecimal::from_str(ad.value.as_str()),
                "slippage" => ret.slippage = BigDecimal::from_str(ad.value.as_str()),
                "time_limit" => ret.time_limit = parse_duration::parse(ad.value.as_str()),
                &_ => {}
            }
        }

        ret
    }

    pub fn selector() -> H256 {
        keccak(abi::encode(&[Token::Bytes(Vec::from(
            APP_SELECTOR.as_bytes(),
        ))]))
        .as_fixed_bytes()
        .into()
    }
}

impl Solver for LimitOrderSolver {
    fn app(&self) -> String {
        return APP_SELECTOR.to_string();
    }
    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error> {
        self.time_limit.clone()
    }
    fn exec_solver_step(&self) -> Result<bool, SolverError> {
        Ok(true)
    }
}
