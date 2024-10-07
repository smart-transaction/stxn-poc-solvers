use crate::{contracts_abi::laminator::AdditionalData, solvers::limit_order::LimitOrderSolver};
use ethers::types::H256;
use parse_duration;
use std::{fmt, fmt::Display, sync::Arc, time::Duration};

pub enum SolverError {
    InvalidParam(String),
    ExecError(String),
}

impl Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverError::InvalidParam(s) => {
                write!(f, "Invalid parameter \"{}\"", s)
            }
            SolverError::ExecError(s) => {
                write!(f, "Execution error, {}", s)
            }
        }
    }
}

pub trait Solver {
    fn app(&self) -> String; 
    fn exec_solver_step(&self) -> Result<bool, SolverError>;
    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error>;
}

pub struct SolverFactory;

pub enum SolverFactoryError {
    UnknownSelector(H256),
}

impl Display for SolverFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverFactoryError::UnknownSelector(s) => {
                write!(f, "UnknownSelector: {}", s)
            }
        }
    }
}

impl SolverFactory {
    pub fn new_solver(
        selector: H256,
        params: &Vec<AdditionalData>,
    ) -> Result<Arc<dyn Solver>, SolverFactoryError> {
        let flash_liquidity_selector = LimitOrderSolver::selector();
        if selector == flash_liquidity_selector.into() {
            return Ok(Arc::new(LimitOrderSolver::new(params)));
        }
        Err(SolverFactoryError::UnknownSelector(selector))
    }
}
