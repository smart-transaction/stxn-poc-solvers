use ethers::{
    abi::AbiEncode,
    types::{Address, H256},
};
use keccak_hash::keccak;
use std::{
    collections::HashMap,
    fmt::{self, Display},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SolverParams<M>
where
    M: Clone,
{
    pub call_breaker_address: Address,
    pub solver_address: Address,
    pub extra_contract_addresses: HashMap<String, Address>,
    pub middleware: Arc<M>,
    pub guard: Arc<Mutex<bool>>,
}

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

pub trait Solver {
    fn app(&self) -> String;
    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error>;
    async fn init_exec(&self);
    async fn exec_solver_step(&self) -> Result<bool, SolverError>;
    async fn final_exec(&self) -> Result<bool, SolverError>;
}

pub fn selector(app: String) -> H256 {
    keccak(app.as_str().encode()).as_fixed_bytes().into()
}
