use ethers::prelude::abigen;

abigen!(
  CallBreaker,
  "./abi_town/CallBreaker.sol/CallBreaker.json",
  derives(serde::Deserialize, serde::Serialize);

  IERC20,
  "./abi_town/IERC20.sol/IERC20.json",
  derives(serde::Deserialize, serde::Serialize);
);
