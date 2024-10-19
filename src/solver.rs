use ethers::types::Address;
use std::{collections::HashMap, sync::Arc};

pub struct SolverParams<M> {
    pub call_breaker_address: Address,
    pub solver_address: Address,
    pub extra_contract_addresses: HashMap<String, Address>,
    pub middleware: Arc<M>,
}
