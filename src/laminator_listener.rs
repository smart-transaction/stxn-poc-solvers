use crate::{
    contracts_abi::laminator::{Laminator, ProxyPushedFilter},
    timer_executor::TimerExecutorFrame,
};
use ethers::{
    providers::{Middleware, StreamExt}, types::U64,
};
use fatal::fatal;

pub struct LaminatedProxyListener<M> {
    laminator_contract: Laminator<M>,
    executor_frame: TimerExecutorFrame<M>,
}

impl<M: Middleware + 'static> LaminatedProxyListener<M> {
    pub fn new(
        laminator_contract: Laminator<M>,
        executor_frame: TimerExecutorFrame<M>,
    ) -> LaminatedProxyListener<M> {
        LaminatedProxyListener::<M> {
            laminator_contract,
            executor_frame,
        }
    }

    pub async fn listen(&mut self, block: U64) {
        let events = self.laminator_contract.event::<ProxyPushedFilter>().from_block(block);
        match events.stream().await {
            Ok(stream) => {
                let mut stream_take = stream.take(1);
                println!("Listening the event ProxyPushed from block {} ...", block);
                while let Some(Ok(proxy_pushed)) = stream_take.next().await {
                    self.executor_frame.start_executor(proxy_pushed);
                }
            }
            Err(err) => {
                fatal!("Error reading events from stream: {}", err);
            }
        };
    }
}
