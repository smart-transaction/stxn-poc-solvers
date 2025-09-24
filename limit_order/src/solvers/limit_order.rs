use crate::{
    contracts_abi::{
        call_breaker::{CallBreaker, CallObject, MevTimeData, UserObjective},
        ierc20::{ApproveCall, IERC20Calls},
        UserObjectivePushedFilter,
    },
    solver::{selector, Solver, SolverError, SolverParams, SolverResponse},
};
use ethers::{
    abi::{self, AbiDecode, AbiEncode, Token},
    core::abi::ethabi::ethereum_types::FromDecStrErr,
    prelude::abigen,
    providers::Middleware,
    signers::LocalWallet,
    types::{Address, Bytes, H256, U256},
    utils::{hash_message, keccak256, parse_units},
};
use fixed_hash::rustc_hex::FromHexError;
use parse_duration;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

abigen!(
    FlashLoan,
    "./abi_town/MockFlashLoan.sol/MockFlashLoan.json";

    SwapPool,
    "./abi_town/MockDaiWethPool.sol/MockDaiWethPool.json";
);

pub const APP_SELECTOR: &str = "FLASHLIQUIDITY.LIMITORDER";
pub const FLASH_LOAN_NAME: &str = "FLASH_LOAN";
pub const SWAP_POOL_NAME: &str = "SWAP_POOL";

pub struct LimitOrderSolver<M> {
    // Solver address
    _solver_address: Address, // To be used after fixing associated data

    // Contract addresses to be called.
    call_breaker_address: Address,
    flash_loan_address: Address,
    swap_pool_address: Address,

    // Contracts that are to be called.
    call_breaker_contract: CallBreaker<M>,
    swap_pool_contract: SwapPool<M>,

    // Limit order params
    pub give_token: Result<Address, FromHexError>,
    pub take_token: Result<Address, FromHexError>,
    buy_price: Result<U256, FromDecStrErr>,
    slippage: Result<U256, FromDecStrErr>,
    time_limit: Result<Duration, parse_duration::parse::Error>,

    pub user_objective: UserObjective,
    wallet: LocalWallet,

    // Transaction guard
    guard: Arc<Mutex<bool>>,
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
        let mut res = self.provider.encode();
        res.extend(self.amount_a.encode());
        res.extend(self.amount_b.encode());
        res
    }
}

impl<M: Middleware + Clone> LimitOrderSolver<M> {
    pub fn new(
        event: UserObjectivePushedFilter,
        params: SolverParams<M>,
    ) -> Result<LimitOrderSolver<M>, SolverError> {
        println!("Event received: {}", event);
        let expected_app_id = selector(APP_SELECTOR.to_string());
        let event_app_id: H256 = event.app_id;
        if event_app_id != expected_app_id {
            // Convert TxHash to H256 for error reporting
            return Err(SolverError::MisleadingSelector(event_app_id));
        }
        let flash_loan_address = params.extra_contract_addresses.get(FLASH_LOAN_NAME);
        if let None = flash_loan_address {
            return Err(SolverError::ParamError(
                "missing address for contract FLASH_LOAN".to_string(),
            ));
        }
        let swap_pool_address = params.extra_contract_addresses.get(SWAP_POOL_NAME);
        if let None = swap_pool_address {
            return Err(SolverError::ParamError(
                "missing adsdress for contract SWAP_POOL".to_string(),
            ));
        }
        let mut ret = LimitOrderSolver {
            user_objective: event.user_objective.clone(),
            call_breaker_address: params.call_breaker_address,
            _solver_address: params.solver_address,
            flash_loan_address: *flash_loan_address.unwrap(),
            swap_pool_address: *swap_pool_address.unwrap(),
            call_breaker_contract: CallBreaker::new(
                params.call_breaker_address,
                params.middleware.clone(),
            ),
            swap_pool_contract: SwapPool::new(
                *swap_pool_address.unwrap(),
                params.middleware.clone(),
            ),
            give_token: Result::Err(FromHexError::InvalidHexLength),
            take_token: Result::Err(FromHexError::InvalidHexLength),
            buy_price: Result::Err(FromDecStrErr::InvalidLength),
            slippage: Result::Err(FromDecStrErr::InvalidLength),
            time_limit: Result::Err(parse_duration::parse::Error::NoValueFound(
                "Uninitialized value".to_string(),
            )),
            wallet: params.wallet.clone(),
            guard: params.guard.clone(),
        };
        // Extract parameters.
        for ad in &event.mev_time_data {
            match ad.key {
                key_bytes
                    if key_bytes
                        == keccak256(abi::encode(&[ethers::abi::Token::String(
                            "give_token".to_string(),
                        )])) =>
                {
                    let hex_string = format!("0x{}", hex::encode(&ad.value));
                    let address: Address = hex_string.parse().unwrap();
                    ret.give_token = Ok(address);
                }
                key_bytes
                    if key_bytes
                        == keccak256(abi::encode(&[ethers::abi::Token::String(
                            "take_token".to_string(),
                        )])) =>
                {
                    let hex_string = format!("0x{}", hex::encode(&ad.value));
                    let address: Address = hex_string.parse().unwrap();
                    ret.take_token = Ok(address);
                }
                key_bytes
                    if key_bytes
                        == keccak256(abi::encode(&[ethers::abi::Token::String(
                            "buy_price".to_string(),
                        )])) =>
                {
                    if let Ok(tokens) = abi::decode(&[abi::ParamType::Uint(256)], &ad.value) {
                        if let Some(Token::Uint(price)) = tokens.first() {
                            ret.buy_price = Ok(*price);
                        }
                    }
                }
                key_bytes
                    if key_bytes
                        == keccak256(abi::encode(&[ethers::abi::Token::String(
                            "slippage".to_string(),
                        )])) =>
                {
                    if let Ok(tokens) = abi::decode(&[abi::ParamType::Uint(256)], &ad.value) {
                        if let Some(Token::Uint(slippage)) = tokens.first() {
                            ret.slippage = Ok(*slippage);
                        }
                    }
                }
                key_bytes
                    if key_bytes
                        == keccak256(abi::encode(&[ethers::abi::Token::String(
                            "time_limit".to_string(),
                        )])) =>
                {
                    let decoded_string = String::decode(&ad.value).unwrap();

                    // Then parse the string to duration
                    let duration = parse_duration::parse(&decoded_string).unwrap();

                    ret.time_limit = Ok(duration);
                }
                _ => println!("Unknown key: {:?}", ad.key),
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
}

impl<M: Middleware> Solver for LimitOrderSolver<M> {
    fn app(&self) -> String {
        return APP_SELECTOR.to_string();
    }

    fn time_limit(&self) -> Result<Duration, parse_duration::parse::Error> {
        self.time_limit.clone()
    }

    async fn exec_solver_step(&self) -> Result<SolverResponse, SolverError> {
        if let Err(err) = &self.buy_price {
            return Err(SolverError::ExecError(err.to_string()));
        }
        // Check the price
        println!("Checking the price");
        match self.swap_pool_contract.get_price_of_weth().call().await {
            Ok(current_price) => {
                let desired_price = *self.buy_price.as_ref().ok().unwrap();
                println!("Current price: {}", current_price);
                println!("Desired price: {}", desired_price);
                if current_price > desired_price {
                    return Ok(SolverResponse {
                        succeeded: false,
                        message: format!(
                            "The current price {} is higher than the desired {}",
                            current_price, desired_price
                        ),
                    });
                }
            }
            Err(err) => {
                return Err(SolverError::ExecError(err.to_string()));
            }
        }
        Ok(SolverResponse {
            succeeded: true,
            message: "Price conditions are met".to_string(),
        })
    }

    async fn final_exec(&self) -> Result<SolverResponse, SolverError> {
        let hardcoded_weth_liquidity = 100;
        let hardcoded_dai_liquidity = 1000;
        let dai_liquidity_wei = parse_units(hardcoded_dai_liquidity, "ether").ok().unwrap();
        let weth_liquidity_wei = parse_units(hardcoded_weth_liquidity, "ether").ok().unwrap();
        let call_objects = vec![
            // CallObject 0: DAI approve for swap pool
            CallObject {
                amount: 0.into(),
                addr: self.give_token.ok().unwrap(), // DAI address
                gas: 1000000.into(),
                callvalue: IERC20Calls::Approve(ApproveCall {
                    spender: self.swap_pool_address,
                    amount: dai_liquidity_wei.into(),
                })
                .encode()
                .into(),
                salt: 1.into(),
                returnvalue: Bytes::new(),
                skippable: false,
                verifiable: true,
                expose_return: false,
            },
            // CallObject 1: WETH approve for swap pool
            CallObject {
                amount: 0.into(),
                addr: self.take_token.ok().unwrap(), // WETH address
                gas: 1000000.into(),
                callvalue: IERC20Calls::Approve(ApproveCall {
                    spender: self.swap_pool_address,
                    amount: weth_liquidity_wei.into(),
                })
                .encode()
                .into(),
                salt: 1.into(),
                returnvalue: Bytes::new(),
                skippable: false,
                verifiable: true,
                expose_return: false,
            },
            // CallObject 2: Provide liquidity to DAI/WETH pool
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
                salt: 1.into(),
                returnvalue: Bytes::new(),
                skippable: false,
                verifiable: true,
                expose_return: false,
            },
            // CallObject 3: Check slippage (future call)
            CallObject {
                amount: 0.into(),
                addr: self.swap_pool_address,
                gas: 10000000.into(),
                callvalue: SwapPoolCalls::CheckSlippage(CheckSlippageCall {
                    max_deviation_percentage: *self.slippage.as_ref().ok().unwrap(),
                })
                .encode()
                .into(),
                salt: 0.into(),
                returnvalue: Bytes::new(),
                skippable: false,
                verifiable: true,
                expose_return: true,
            },
            // CallObject 4: Withdraw liquidity from DAI/WETH pool
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
                salt: 1.into(),
                returnvalue: Bytes::new(),
                skippable: false,
                verifiable: true,
                expose_return: false,
            },
        ];

        let user_objectives = vec![
            self.user_objective.clone(),
            UserObjective {
                app_id: Bytes::from(
                    selector("FLASHLIQUIDITY.LIMITORDER".to_string())
                        .as_bytes()
                        .to_vec(),
                ),
                nonce: 0.into(),
                tip: 0.into(),
                chain_id: 0.into(),
                max_fee_per_gas: 0.into(),
                max_priority_fee_per_gas: 0.into(),
                sender: self._solver_address,
                signature: solver_signature(
                    0.into(),
                    &self._solver_address,
                    &call_objects,
                    &self.wallet,
                )
                .unwrap(),
                call_objects,
            },
        ];

        // Setting order of execution
        let order_of_execution = vec![
            U256::from(1),
            U256::from(2),
            U256::from(3),
            U256::from(0),
            U256::from(4),
            U256::from(5),
        ];

        // Return values for each call
        let returns_bytes = vec![
            abi::encode(&[Token::Bool(true)]).into(),
            abi::encode(&[Token::Bool(true)]).into(),
            Bytes::new(),
            Bytes::new(),
            Bytes::new(),
            Bytes::new(),
        ];

        // Create MevTimeData struct
        let mev_time_data = MevTimeData {
            validator_signature: Bytes::new(),
            mev_time_data_values: vec![],
        };

        {
            let _guard = self.guard.lock().await;
            match self
                .call_breaker_contract
                .execute_and_verify(
                    user_objectives,
                    returns_bytes,
                    order_of_execution,
                    mev_time_data,
                )
                .gas(5_000_000)
                .send()
                .await
            {
                Ok(pending) => {
                    println!("Transaction is sent, txhash: {}", pending.tx_hash());
                    match pending.await {
                        Ok(receipt) => {
                            if let Some(receipt) = receipt {
                                if let Some(status) = receipt.status {
                                    return Ok(SolverResponse {
                                        succeeded: status != 0.into(),
                                        message: format!("Transaction status: {}", status),
                                    });
                                }
                            }
                            return Ok(SolverResponse {
                                succeeded: false,
                                message: "transaction status wasn't received".to_string(),
                            });
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
        };
    }
}

// Generate solver Signature
fn solver_signature(
    nonce: U256,
    sender: &Address,
    call_objects: &Vec<CallObject>,
    wallet: &LocalWallet,
) -> Result<Bytes, SolverError> {
    // Convert CallObjects to Token tuples for encoding
    let call_tokens: Vec<Token> = call_objects
        .iter()
        .map(|call_obj| {
            Token::Tuple(vec![
                Token::Uint(call_obj.salt),
                Token::Uint(call_obj.amount),
                Token::Uint(call_obj.gas),
                Token::Address(call_obj.addr),
                Token::Bytes(call_obj.callvalue.clone().to_vec()),
                Token::Bytes(call_obj.returnvalue.clone().to_vec()),
                Token::Bool(call_obj.skippable),
                Token::Bool(call_obj.verifiable),
                Token::Bool(call_obj.expose_return),
            ])
        })
        .collect();

    // Match the contract's signature verification exactly
    let encoded_call_objects = abi::encode(&[Token::Array(call_tokens)]);
    let encoded_data = abi::encode(&[
        Token::Uint(nonce),
        Token::Address(*sender),
        Token::Bytes(encoded_call_objects),
    ]);

    let hash_bytes = keccak256(&encoded_data);
    let hash = H256::from_slice(&hash_bytes);

    // Ethereum-specific message prefix (EIP-191)
    let eth_hash = hash_message(hash);

    match wallet.sign_hash(eth_hash) {
        Ok(sig) => {
            // Convert into 65-byte compact form
            let compact: [u8; 65] = sig.to_vec().try_into().map_err(|_| {
                SolverError::ExecError("Failed to convert signature to compact form".to_string())
            })?;
            Ok(Bytes::from(compact.to_vec()))
        }
        Err(err) => Err(SolverError::ExecError(format!(
            "Failed to sign hash: {}",
            err
        ))),
    }
}
