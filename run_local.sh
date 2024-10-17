CHAIN_ID=21363
WS_CHAIN_URL=wss://service.lestnet.org:8888/
LAMINATOR_ADDRESS=0xF8f81f532d1f2787BECd3ecD0734e9BEd1241313
CALL_BREAKER_ADDRESS=0xBc9b024028C67E147829824bB767aa780958EEAa
FLASH_LOAN_ADDRESS=0x6341B9Bf738adB9E4224966615bAa8f49D328245
SWAP_POOL_ADDRESS=0xB8113C66Da8672A1Bee76bc6a2d9ea82c8062f49
WALLET_PRIVATE_KEY=$(gcloud secrets versions access 1 --secret="WALLET_PRIVATE_KEY_DEV")
TICK_SECS=5
TICK_NANOS=0

cargo run \
  -- \
  --chain-id=${CHAIN_ID} \
  --ws-chain-url=${WS_CHAIN_URL} \
  --laminator-address=${LAMINATOR_ADDRESS} \
  --call-breaker-address=${CALL_BREAKER_ADDRESS} \
  --flash-loan-address=${FLASH_LOAN_ADDRESS} \
  --swap-pool-address=${SWAP_POOL_ADDRESS} \
  --wallet-private-key=${WALLET_PRIVATE_KEY} \
  --tick-secs=${TICK_SECS} \
  --tick-nanos=${TICK_NANOS}
