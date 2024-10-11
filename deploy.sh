# Full stxn solver setup on a clean Linux machine.
#
# Pre-reqs:
# 1. Linux machine: Debian/Ubuntu/...
# 2. setup.sh file from our setup folder locally in a local folder
#    (pulled from Github or otherwise).

# Vars init
PORT=
CHAIN_ID=
WS_CHAIN_URL=
LAMINATOR_ADDRESS=
CALL_BREAKER_ADDRESS=
FLASH_LOAN_ADDRESS=
SWAP_POOL_ADDRESS=
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
        PORT=8080
        CHAIN_ID=21363
        WS_CHAIN_URL=wss://service.lestnet.org:8888/
        LAMINATOR_ADDRESS=0x92972cE554A368Da0bF58E5F32c72B7565Ef59d7
        CALL_BREAKER_ADDRESS=0xBc9b024028C67E147829824bB767aa780958EEAa
        FLASH_LOAN_ADDRESS=0x09E85342ee23cEfb4AfB277e700A184d4d6C32e1
        SWAP_POOL_ADDRESS=0xF6c98DE27292FAeb3d34632ddA2b937358c0a34E
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
WALLET_PRIVATE_KEY=\$(gcloud secrets versions access 1 --secret="WALLET_PRIVATE_KEY_${SECRET_SUFFIX}")

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
SOLVER_DOCKER_IMAGE="${DOCKER_PREFIX}/stxn-solver-image:${OPT}"

# Create docker-compose.yml file.
cat >docker-compose.yml << COMPOSE
version: '3'

services:
  solver:
    container_name: stxn_solver
    image: ${SOLVER_DOCKER_IMAGE}
    environment:
      - PORT=${PORT}
      - CHAIN_ID=${CHAIN_ID}
      - WS_CHAIN_URL=${WS_CHAIN_URL}
      - LAMINATOR_ADDRESS=${LAMINATOR_ADDRESS}
      - CALL_BREAKER_ADDRESS=${CALL_BREAKER_ADDRESS}
      - FLASH_LOAN_ADDRESS=${FLASH_LOAN_ADDRESS}
      - SWAP_POOL_ADDRESS=${SWAP_POOL_ADDRESS}
      - WALLET_PRIVATE_KEY=\${WALLET_PRIVATE_KEY}
      - TICK_SECS=${TICK_SECS}
      - TICK_NANOS=${TICK_NANOS}
    ports:
      - 8080:8080

COMPOSE

set -e

# Pull images:
docker pull ${SOLVER_DOCKER_IMAGE}

# Start our docker images.
./up.sh
