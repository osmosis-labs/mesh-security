use cosmwasm_std::{to_binary, Addr, Coin};
use cw_multi_test::{App as MtApp, AppResponse};
use mesh_apis::{converter_api::RewardInfo, ibc::AddValidator};
use mesh_sync::Tx;
use mesh_vault::contract::multitest_utils::VaultContractProxy;
use sylvia::multitest::App;

use crate::{
    contract::multitest_utils::ExternalStakingContractProxy, error::ContractError,
    msg::ReceiveVirtualStake, test_methods_impl::test_utils::TestMethods as _,
};

macro_rules! assert_rewards {
    ($contract:expr, $user:expr, $validator:expr, $expected:expr) => {
        let rewards = $contract
            .pending_rewards($user.to_string(), $validator.to_string())
            .unwrap()
            .rewards;
        let expected = $expected;
        let actual = rewards.amount.u128();
        assert_eq!(
            actual, expected,
            "expected {} reward tokens, found: {}",
            expected, actual
        );
    };
}

pub(crate) use assert_rewards;

#[track_caller]
pub(crate) fn get_last_external_staking_pending_tx_id(
    contract: &ExternalStakingContractProxy<MtApp>,
) -> Option<u64> {
    let txs = contract.all_pending_txs_desc(None, None).unwrap().txs;
    txs.first().map(Tx::id)
}

pub(crate) trait AppExt {
    fn new_with_balances(balances: &[(&str, &[Coin])]) -> Self;
}

impl AppExt for App<MtApp> {
    #[track_caller]
    fn new_with_balances(balances: &[(&str, &[Coin])]) -> Self {
        let app = MtApp::new(|router, _api, storage| {
            for (addr, coins) in balances {
                router
                    .bank
                    .init_balance(storage, &Addr::unchecked(*addr), coins.to_vec())
                    .unwrap();
            }
        });
        Self::new(app)
    }
}

type Vault<'app> = VaultContractProxy<'app, MtApp>;
type Contract<'app> = ExternalStakingContractProxy<'app, MtApp>;

pub(crate) trait ContractExt {
    fn activate_validators<const N: usize>(
        &self,
        validators: [&'static str; N],
    ) -> [&'static str; N];

    fn distribute_batch(
        &self,
        caller: impl AsRef<str>,
        denom: impl Into<String>,
        rewards: &[(&str, u128)],
    ) -> Result<AppResponse, ContractError>;
}

impl ContractExt for Contract<'_> {
    #[track_caller]
    fn activate_validators<const N: usize>(
        &self,
        validators: [&'static str; N],
    ) -> [&'static str; N] {
        for val in validators {
            let activate = AddValidator::mock(val);
            self.test_methods_proxy()
                .test_set_active_validator(activate)
                .call("test")
                .unwrap();
        }

        validators
    }

    #[track_caller]
    fn distribute_batch(
        &self,
        caller: impl AsRef<str>,
        denom: impl Into<String>,
        rewards: &[(&str, u128)],
    ) -> Result<AppResponse, ContractError> {
        let rewards: Vec<_> = rewards
            .iter()
            .map(|(validator, amount)| reward_info(*validator, *amount))
            .collect();

        self.test_methods_proxy()
            .test_distribute_rewards_batch(denom.into(), rewards)
            .call(caller.as_ref())
    }
}

pub(crate) trait VaultExt {
    fn stake(&self, contract: &Contract, user: &str, validator: impl Into<String>, coin: Coin);
}

impl VaultExt for Vault<'_> {
    #[track_caller]
    fn stake(&self, contract: &Contract, user: &str, validator: impl Into<String>, coin: Coin) {
        self.stake_remote(
            contract.contract_addr.to_string(),
            coin,
            to_binary(&ReceiveVirtualStake {
                validator: validator.into(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

        // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
        // This should be through `IbcPacketAckMsg`
        let last_external_staking_tx = get_last_external_staking_pending_tx_id(contract).unwrap();
        contract
            .test_methods_proxy()
            .test_commit_stake(last_external_staking_tx)
            .call("test")
            .unwrap();
    }
}

fn reward_info(validator: impl Into<String>, reward: u128) -> RewardInfo {
    RewardInfo {
        validator: validator.into(),
        reward: reward.into(),
    }
}
