use ethers::prelude::abigen;

abigen!(
  Laminator,
  "./abi_town/Laminator.sol/Laminator.json",
  derives(serde::Deserialize, serde::Serialize);

  CallBreaker,
  "./abi_town/CallBreaker.sol/CallBreaker.json";
);
