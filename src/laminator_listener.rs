use ethers::{
    abi::Address,
    providers::{Middleware, StreamExt},
    types::U64,
};
use fatal::fatal;
use std::sync::Arc;

use crate::{
    contracts_abi::laminator::{Laminator, ProxyPushedFilter},
    timer_executor::TimerExecutorFrame,
};

pub struct LaminatorListener<M> {
    laminator_address: Address,
    middleware: Arc<M>,
    executor_frame: TimerExecutorFrame<M>,
}

impl<M: Middleware + 'static> LaminatorListener<M> {
    pub fn new(
        laminator_address: Address,
        middleware: Arc<M>,
        executor_frame: TimerExecutorFrame<M>,
    ) -> LaminatorListener<M> {
        LaminatorListener::<M> {
            laminator_address,
            middleware,
            executor_frame,
        }
    }

    pub async fn listen(&mut self, block: U64) {
        let laminator_contract = Laminator::new(self.laminator_address, self.middleware.clone());
        let events = laminator_contract
            .event::<ProxyPushedFilter>()
            .from_block(block);
        loop {
            match events.stream().await {
                Ok(stream) => {
                    let mut stream_take = stream.take(10);
                    println!("Listening the event ProxyPushed from block {} ...", block);
                    while let Some(Ok(proxy_pushed)) = stream_take.next().await {
                        self.executor_frame.start_executor(proxy_pushed).await;
                    }
                }
                Err(err) => {
                    fatal!("Error reading events from stream: {}", err);
                }
            }
        }
    }
}
