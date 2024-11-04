PORT=8888
CHAIN_ID=21363
WS_CHAIN_URL=wss://service.lestnet.org:8888/
LAMINATOR_ADDRESS=0x36aB7A6ad656BC19Da2D5Af5b46f3cf3fc47274D
CALL_BREAKER_ADDRESS=0x23912387357621473Ff6514a2DC20Df14cd72E7f
KITN_DISBURSEMENT_SCHEDULER_ADDRESS=0x7E485Fd55CEdb1C303b2f91DFE7695e72A537399
TICK_SECS=5
TICK_NANOS=0

PROJECT_NAME="solver-438012"
CURRENT_PROJECT=$(gcloud config get project)
if [ "${PROJECT_NAME}" != "${CURRENT_PROJECT}" ]; then
  gcloud auth login
  gcloud config set project ${PROJECT_NAME}
fi

CLEANAPP_SCHEDULER_WALLET_PRIVATE_KEY=$(gcloud secrets versions access 1 --secret="KITN_PRIVATE_KEY_DEV")

cargo run \
  -- \
  --port=${PORT} \
  --chain-id=${CHAIN_ID} \
  --ws-chain-url=${WS_CHAIN_URL} \
  --laminator-address=${LAMINATOR_ADDRESS} \
  --call-breaker-address=${CALL_BREAKER_ADDRESS} \
  --kitn-disbursement-scheduler-address=${KITN_DISBURSEMENT_SCHEDULER_ADDRESS} \
  --cleanapp-wallet-private-key=${CLEANAPP_SCHEDULER_WALLET_PRIVATE_KEY} \
  --tick-secs=${TICK_SECS} \
  --tick-nanos=${TICK_NANOS}
