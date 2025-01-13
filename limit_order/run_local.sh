PORT=9999
CHAIN_ID=21363
WS_CHAIN_URL=wss://service.lestnet.org:8888/
LAMINATOR_ADDRESS=0x36aB7A6ad656BC19Da2D5Af5b46f3cf3fc47274D
CALL_BREAKER_ADDRESS=0x23912387357621473Ff6514a2DC20Df14cd72E7f
FLASH_LOAN_ADDRESS=0xA04bABcCbcf9B9E51eE4954DB223E34691F5F65D
SWAP_POOL_ADDRESS=0xD68B5dd90022f9871913198285cce9d90AAcCD62
TICK_SECS=5
TICK_NANOS=0

PROJECT_NAME="solver-438012"
CURRENT_PROJECT=$(gcloud config get project)
if [ "${PROJECT_NAME}" != "${CURRENT_PROJECT}" ]; then
  gcloud auth login
  gcloud config set project ${PROJECT_NAME}
fi

LIMIT_ORDER_WALLET_PRIVATE_KEY=$(gcloud secrets versions access 1 --secret="LOCAL_LESTNET_WALLET_PRIVATE_KEY_DEV")

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
