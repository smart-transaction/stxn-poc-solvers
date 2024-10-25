PORT=9999
CHAIN_ID=21363
WS_CHAIN_URL=wss://service.lestnet.org:8888/
LAMINATOR_ADDRESS=0x3f0f0a5568b5627D4525291c2ca0aCFd0A50773a
CALL_BREAKER_ADDRESS=0x807801255e30996561520CDBC63ffd9bCAa0b57D
FLASH_LOAN_ADDRESS=0x8d88e3C21d756D35350934e1D41027bF77131395
SWAP_POOL_ADDRESS=0xBD62109bd7D732c2169BA8578893D0cfAa7a3bAc
LIMIT_ORDER_WALLET_PRIVATE_KEY=$(gcloud secrets versions access 1 --secret="LOCAL_LESTNET_WALLET_PRIVATE_KEY_DEV")
TICK_SECS=5
TICK_NANOS=0

cargo run \
  -- \
  --port=${PORT} \
  --chain-id=${CHAIN_ID} \
  --ws-chain-url=${WS_CHAIN_URL} \
  --laminator-address=${LAMINATOR_ADDRESS} \
  --call-breaker-address=${CALL_BREAKER_ADDRESS} \
  --flash-loan-address=${FLASH_LOAN_ADDRESS} \
  --swap-pool-address=${SWAP_POOL_ADDRESS} \
  --limit-order-wallet-private-key=${LIMIT_ORDER_WALLET_PRIVATE_KEY} \
  --tick-secs=${TICK_SECS} \
  --tick-nanos=${TICK_NANOS}
