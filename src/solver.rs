use ethers::types::Address;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub struct SolverParams<M>
where
    M: Clone,
{
    pub call_breaker_address: Address,
    pub solver_address: Address,
    pub extra_contract_addresses: HashMap<String, Address>,
    pub middleware: Arc<M>,
}
