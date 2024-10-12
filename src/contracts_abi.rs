use ethers::prelude::abigen;

abigen!(
  CallBreaker,
  "./abi_town/CallBreaker.sol/CallBreaker.json",
  methods {
    executeAndVerify(bytes, bytes, bytes, bytes, bytes) as execute_and_verify_with_flashloan
  },
  derives(serde::Deserialize, serde::Serialize);

  IERC20,
  "./abi_town/IERC20.sol/IERC20.json";

  LaminatedProxy,
  "./abi_town/LaminatedProxy.sol/LaminatedProxy.json",
  derives(serde::Deserialize, serde::Serialize);

  Laminator,
  "./abi_town/Laminator.sol/Laminator.json",
  derives(serde::Deserialize, serde::Serialize);
);
