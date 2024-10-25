# Turn up solver.

# Secrets
cat >.env << ENV
WALLET_PRIVATE_KEY=$(gcloud secrets versions access 1 --secret="WALLET_PRIVATE_KEY_DEV")

ENV

sudo docker-compose up -d --remove-orphans

rm -f .env

