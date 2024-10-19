use std::{collections::HashMap, sync::Arc};

use ethers::types::Address;

use crate::contracts_abi::ProxyPushedFilter;

pub struct SolverParams<M> {
  pub event: ProxyPushedFilter,
  pub call_breaker_address: Address,
  pub solver_address: Address,
  pub extra_contract_addresses: HashMap<String, Address>,
  pub middleware: Arc<M>,
}