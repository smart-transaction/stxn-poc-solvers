use chrono::{DateTime, Utc};
use ethers::types::Address;
use std::{
    fmt::{self, Display},
    sync::Arc,
};

#[derive(Clone)]
pub struct SolverParams<M>
where
    M: Clone,
{
    pub call_breaker_address: Address,
    pub middleware: Arc<M>,
}

pub struct SolverResponse {
    pub succeeded: bool,
    pub message: String,
    pub remaining_secs: i64,
}

#[derive(Clone, Debug)]
pub enum SolverError {
    ParamError(String),
    ExecError(String),
}

impl Display for SolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    fn schedule_time(&self) -> Result<DateTime<Utc>, SolverError>;
    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError>;
    async fn final_exec(&self) -> Result<SolverResponse, SolverError>;
}
