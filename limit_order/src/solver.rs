use ethers::{
    abi::{self, Token},
    signers::LocalWallet,
    types::{Address, H256},
    utils::keccak256,
};
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
    pub wallet: LocalWallet,
}

pub struct SolverResponse {
    pub succeeded: bool,
    pub message: String,
}

#[derive(Debug)]
pub enum SolverError {
    MisleadingSelector(H256),
    ParamError(String),
    ExecError(String),
}

impl Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverError::MisleadingSelector(s) => {
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
    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError>;
    async fn final_exec(&self) -> Result<SolverResponse, SolverError>;
}

pub fn selector(app: String) -> H256 {
    let encoded = abi::encode(&[Token::String(app)]);
    let hash = keccak256(&encoded);
    let bytes32 = H256::from_slice(&hash);
    return bytes32;
}
