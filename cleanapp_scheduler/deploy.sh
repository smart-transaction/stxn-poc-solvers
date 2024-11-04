# Full stxn solver setup on a clean Linux machine.
#
# Pre-reqs:
# 1. Linux machine: Debian/Ubuntu/...
# 2. setup.sh file from our setup folder locally in a local folder
#    (pulled from Github or otherwise).

set -e

# Vars init
CHAIN_ID=
WS_CHAIN_URL=
LAMINATOR_ADDRESS=
CALL_BREAKER_ADDRESS=
KITN_DISBURSEMENT_SCHEDULER_ADDRESS=
TICK_SECS=
TICK_NANOS=

# Choose the environment
PS3="Please choose the environment: "
options=("dev" "prod" "quit")
select OPT in "${options[@]}"
do
  case ${OPT} in
    "dev")
        echo "Using dev environment"
        CHAIN_ID=21363
        WS_CHAIN_URL=wss://service.lestnet.org:8888/
        LAMINATOR_ADDRESS=0x36aB7A6ad656BC19Da2D5Af5b46f3cf3fc47274D
        CALL_BREAKER_ADDRESS=0x23912387357621473Ff6514a2DC20Df14cd72E7f
        KITN_DISBURSEMENT_SCHEDULER_ADDRESS=0x7E485Fd55CEdb1C303b2f91DFE7695e72A537399
        TICK_SECS=1
        TICK_NANOS=0
        break
        ;;
    "prod")
        echo "Prod environment is not implemented"
        exit
        ;;
    "quit")
        exit
        ;;
    *) echo "invalid option $REPLY";;
  esac
done

SECRET_SUFFIX=$(echo ${OPT} | tr '[a-z]' '[A-Z]')

# Create necessary files.
cat >up.sh << UP
# Turn up solver.

# Secrets
cat >.env << ENV
CLEANAPP_SCHEDULER_WALLET_PRIVATE_KEY=\$(gcloud secrets versions access 1 --secret="KITN_PRIVATE_KEY_${SECRET_SUFFIX}")

ENV

sudo docker-compose up -d --remove-orphans

rm -f .env

UP

sudo chmod a+x up.sh

cat >down.sh << DOWN
# Turn down solver.
sudo docker-compose down
DOWN
sudo chmod a+x down.sh

PROJECT_NAME="solver-438012"
DOCKER_IMAGE="solver-docker-repo/stxn-solver-image"


# Docker images
DOCKER_LOCATION="us-central1-docker.pkg.dev"
DOCKER_PREFIX="${DOCKER_LOCATION}/solver-438012/solver-docker-repo"
SOLVER_DOCKER_IMAGE="${DOCKER_PREFIX}/stxn-cleanapp-solver-image:${OPT}"

# Create docker-compose.yml file.
cat >docker-compose.yml << COMPOSE
version: '3'

services:
  solver:
    container_name: stxn_cleanapp_solver
    image: ${SOLVER_DOCKER_IMAGE}
    environment:
      - CHAIN_ID=${CHAIN_ID}
      - WS_CHAIN_URL=${WS_CHAIN_URL}
      - LAMINATOR_ADDRESS=${LAMINATOR_ADDRESS}
      - CALL_BREAKER_ADDRESS=${CALL_BREAKER_ADDRESS}
      - FLASH_LOAN_ADDRESS=${FLASH_LOAN_ADDRESS}
      - KITN_DISBURSEMENT_SCHEDULER_ADDRESS=${KITN_DISBURSEMENT_SCHEDULER_ADDRESS}
      - SWAP_POOL_ADDRESS=${SWAP_POOL_ADDRESS}
      - CLEANAPP_SCHEDULER_WALLET_PRIVATE_KEY=\${CLEANAPP_SCHEDULER_WALLET_PRIVATE_KEY}
      - TICK_SECS=${TICK_SECS}
      - TICK_NANOS=${TICK_NANOS}
    ports:
      - 8888:8888

COMPOSE

set -e

# Pull images:
docker pull ${SOLVER_DOCKER_IMAGE}

# Start our docker images.
./up.sh
