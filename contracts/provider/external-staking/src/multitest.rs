mod utils;

use anyhow::Result as AnyResult;

use cosmwasm_std::{coin, coins, to_json_binary, Decimal, Uint128};
use cw_multi_test::App as MtApp;
use mesh_native_staking::contract::sv::mt::CodeId as NativeStakingCodeId;
use mesh_native_staking::contract::sv::InstantiateMsg as NativeStakingInstantiateMsg;
use mesh_native_staking_proxy::mock::sv::mt::CodeId as NativeStakingProxyCodeId;
use mesh_vault::mock::sv::mt::{CodeId as VaultCodeId, VaultMockProxy};
use mesh_vault::mock::VaultMock;
use mesh_vault::msg::{LocalStakingInfo, StakingInitInfo};

use mesh_sync::ValueRange;

use sylvia::multitest::{App, Proxy};

use crate::contract::sv::mt::ExternalStakingContractProxy;
use crate::test_methods::sv::mt::TestMethodsProxy;
use mesh_apis::cross_staking_api::sv::mt::CrossStakingApiProxy;

use crate::contract::sv::mt::CodeId;
use crate::contract::ExternalStakingContract;
use crate::error::ContractError;
use crate::msg::{AuthorizedEndpoint, ReceiveVirtualStake, StakeInfo, ValidatorPendingRewards};
use crate::state::{SlashRatio, Stake};
use utils::{
    assert_rewards, get_last_external_staking_pending_tx_id, AppExt as _, ContractExt as _,
    VaultExt as _,
};

const OSMO: &str = "osmo";
const STAR: &str = "star";

/// 10% slashing on the remote chain
const SLASHING_PERCENTAGE: u64 = 10;
/// 5% slashing on the local chain (so we can differentiate in future tests)
const LOCAL_SLASHING_PERCENTAGE_DSIGN: u64 = 5;
const LOCAL_SLASHING_PERCENTAGE_OFFLINE: u64 = 5;

// Shortcut setuping all needed contracts
//
// Returns vault and external staking proxies
fn setup<'app>(
    app: &'app App<MtApp>,
    owner: &'app str,
    unbond_period: u64,
) -> AnyResult<(
    Proxy<'app, MtApp, VaultMock<'app>>,
    Proxy<'app, MtApp, ExternalStakingContract<'app>>,
)> {
    let native_staking_proxy_code = NativeStakingProxyCodeId::store_code(app);
    let native_staking_code = NativeStakingCodeId::store_code(app);
    let vault_code = VaultCodeId::store_code(app);
    let contract_code = CodeId::store_code(app);

    let native_staking_instantiate = NativeStakingInstantiateMsg {
        denom: OSMO.to_owned(),
        proxy_code_id: native_staking_proxy_code.code_id(),
        slash_ratio_dsign: Decimal::percent(LOCAL_SLASHING_PERCENTAGE_DSIGN),
        slash_ratio_offline: Decimal::percent(LOCAL_SLASHING_PERCENTAGE_OFFLINE),
    };

    let staking_init = StakingInitInfo {
        admin: None,
        code_id: native_staking_code.code_id(),
        msg: to_json_binary(&native_staking_instantiate)?,
        label: Some("Native staking".to_owned()),
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), Some(LocalStakingInfo::New(staking_init)))
        .call(owner)?;

    let remote_contact = AuthorizedEndpoint::new("connection-2", "wasm-osmo1foobarbaz");

    let contract = contract_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact,
            SlashRatio {
                double_sign: Decimal::percent(SLASHING_PERCENTAGE),
                offline: Decimal::percent(SLASHING_PERCENTAGE),
            },
        )
        .call(owner)?;

    Ok((vault, contract))
}

#[test]
fn instantiate() {
    let app = App::default();

    let owner = "owner";
    let users = ["user1"];

    let (_, contract) = setup(&app, owner, 100).unwrap();

    let stakes = contract.stakes(users[0].to_owned(), None, None).unwrap();
    assert_eq!(stakes.stakes, []);

    let max_slash = contract.max_slash().unwrap();
    assert_eq!(
        max_slash.slash_ratio_dsign,
        Decimal::percent(SLASHING_PERCENTAGE)
    );
}

#[test]
fn staking() {
    let users = ["user1", "user2"];
    let owner = "owner";

    let app =
        App::new_with_balances(&[(users[0], &coins(300, OSMO)), (users[1], &coins(300, OSMO))]);

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    // Bond tokens
    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(users[0])
        .unwrap();

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(users[1])
        .unwrap();

    /*
    // Fail to stake on non-registered validator
    let msg = to_json_binary(&ReceiveVirtualStake {
        validator: "unknown".to_string(),
    })
    .unwrap();
    println!("START");
    // FIXME: Sylvia panics here, with this line in ExecProxy::call
    //             .map_err(|err| err.downcast().unwrap())
    // Note that the error didn't happen in vault, but in a SubMsg, so this should be some StdError not ContractError...
    let res = vault
        .stake_remote(contract.contract_addr.to_string(), coin(100, OSMO), msg)
        .call(users[0]);
    println!("GOT: {:?}", res);
    assert!(res.is_err());
    */

    vault.stake(&contract, users[0], validators[0], coin(100, OSMO));
    vault.stake(&contract, users[0], validators[1], coin(100, OSMO));
    vault.stake(&contract, users[0], validators[0], coin(100, OSMO));
    vault.stake(&contract, users[1], validators[0], coin(100, OSMO));
    vault.stake(&contract, users[1], validators[1], coin(200, OSMO));

    // All tokens should be only on the vault contract
    assert_eq!(app.app().wrap().query_all_balances(users[0]).unwrap(), []);
    assert_eq!(app.app().wrap().query_all_balances(users[1]).unwrap(), []);
    assert_eq!(
        app.app()
            .wrap()
            .query_all_balances(&vault.contract_addr)
            .unwrap(),
        coins(600, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_all_balances(&contract.contract_addr)
            .unwrap(),
        []
    );

    // Querying for particular stakes
    let stake = contract
        .stake(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(200)));

    let stake = contract
        .stake(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    let stake = contract
        .stake(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    let stake = contract
        .stake(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(200)));

    // Querying fo all the stakes
    let stakes = contract.stakes(users[0].to_owned(), None, None).unwrap();
    assert_eq!(
        stakes.stakes,
        [
            StakeInfo::new(users[0], validators[0], &Stake::from_amount(200u128.into())),
            StakeInfo::new(users[0], validators[1], &Stake::from_amount(100u128.into()))
        ]
    );

    let stakes = contract.stakes(users[1].to_owned(), None, None).unwrap();
    assert_eq!(
        stakes.stakes,
        [
            StakeInfo::new(users[1], validators[0], &Stake::from_amount(100u128.into())),
            StakeInfo::new(users[1], validators[1], &Stake::from_amount(200u128.into()))
        ]
    );
}

#[test]
fn unstaking() {
    let users = ["user1", "user2"];

    let app =
        App::new_with_balances(&[(users[0], &coins(300, OSMO)), (users[1], &coins(300, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(users[0])
        .unwrap();

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(users[1])
        .unwrap();

    vault.stake(&contract, users[0], validators[0], coin(200, OSMO));
    vault.stake(&contract, users[0], validators[1], coin(100, OSMO));
    vault.stake(&contract, users[1], validators[0], coin(300, OSMO));

    // Properly unstake some tokens
    // users[0] unstakes 50 from validators[0] - 150 left staken in 2 batches
    // users[1] usntakes 60 from validators[0] - 240 left staken
    contract
        .unstake(validators[0].to_string(), coin(20, OSMO))
        .call(users[0])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    contract
        .unstake(validators[0].to_string(), coin(30, OSMO))
        .call(users[0])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    contract
        .unstake(validators[0].to_string(), coin(60, OSMO))
        .call(users[1])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Trying some unstakes over what is staken fails
    let err = contract
        .unstake(validators[1].to_string(), coin(110, OSMO))
        .call(users[0])
        .unwrap_err();
    assert_eq!(err, ContractError::NotEnoughStake(100u128.into()));

    let err = contract
        .unstake(validators[0].to_string(), coin(300, OSMO))
        .call(users[1])
        .unwrap_err();
    assert_eq!(err, ContractError::NotEnoughStake(240u128.into()));

    let err = contract
        .unstake(validators[1].to_string(), coin(1, OSMO))
        .call(users[1])
        .unwrap_err();
    assert_eq!(err, ContractError::NotEnoughStake(0u128.into()));

    // Unstaken should be immediately visible on staken amount
    let stake = contract
        .stake(users[0].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(150)));

    let stake = contract
        .stake(users[0].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    let stake = contract
        .stake(users[1].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(240)));

    let stake = contract
        .stake(users[1].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::zero()));

    // But not on vault side
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    // Immediately withdrawing liens
    contract.withdraw_unbonded().call(users[0]).unwrap();
    contract.withdraw_unbonded().call(users[1]).unwrap();

    // Claims still not changed on the vault side
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    // Very short travel in future - too short for claims to release
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(50);
    });

    // Withdrawing liens
    contract.withdraw_unbonded().call(users[0]).unwrap();
    contract.withdraw_unbonded().call(users[1]).unwrap();

    // Claims still not changed on the vault side - withdrawal to early
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    // Adding some more unstakes
    // users[0] unstakes 70 from validators[0] - 80 left staken
    // users[1] unstakes 90 from validators[1] = 10 left staken
    contract
        .unstake(validators[0].to_owned(), coin(70, OSMO))
        .call(users[0])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    contract
        .unstake(validators[1].to_owned(), coin(90, OSMO))
        .call(users[0])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Verify proper stake values
    let stake = contract
        .stake(users[0].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(80)));

    let stake = contract
        .stake(users[0].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(10)));

    let stake = contract
        .stake(users[1].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(240)));

    let stake = contract
        .stake(users[1].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(0)));

    // Another timetravel - just enough for first batch of stakes to release,
    // too early for second batch
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(50);
    });

    // Withdrawing liens
    contract.withdraw_unbonded().call(users[0]).unwrap();
    contract.withdraw_unbonded().call(users[1]).unwrap();

    // Now claims on vault got reduced, but only for first batch amount
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 250);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 240);

    // Moving forward more, passing through second bath pending duration
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(60);
    });

    // Nothing gets released automatically, values just like before
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 250);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 240);

    // Withdrawing liens
    contract.withdraw_unbonded().call(users[0]).unwrap();
    contract.withdraw_unbonded().call(users[1]).unwrap();

    // Now everything is released
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 90);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 240);
}

#[test]
fn immediate_unstake_if_unbonded_validator() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(200, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1"]);

    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();
    vault.stake(&contract, user, validators[0], coin(200, OSMO));

    contract.remove_validator(validators[0]);

    contract
        .unstake(validators[0].to_string(), coin(200, OSMO))
        .call(user)
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();
    contract.withdraw_unbonded().call(user).unwrap();

    let err = vault
        .claim(user.to_string(), contract.contract_addr.to_string())
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&mesh_vault::error::ContractError::NoClaim.to_string()));
}

#[test]
fn immediate_unstake_if_tombstoned_validator() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(200, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1"]);

    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();
    vault.stake(&contract, user, validators[0], coin(200, OSMO));

    contract.tombstone_validator(validators[0]);

    contract
        .unstake(validators[0].to_string(), coin(200, OSMO))
        .call(user)
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();
    contract.withdraw_unbonded().call(user).unwrap();

    let err = vault
        .claim(user.to_string(), contract.contract_addr.to_string())
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&mesh_vault::error::ContractError::NoClaim.to_string()));
}

#[test]
fn distribution() {
    let owner = "owner";
    let users = ["user1", "user2"];
    let remote = ["remote1", "remote2"];

    let app = App::new_with_balances(&[
        (users[0], &coins(600, OSMO)),
        (users[1], &coins(600, OSMO)),
        (owner, &[coin(1000, STAR), coin(1000, OSMO)]),
    ]);

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    // Weights proportion:
    // 2/5 of validators[0] to users[0]
    // 3/5 of validators[0] to users[1]
    // all of validators[1] to users[1]
    vault
        .bond()
        .with_funds(&coins(600, OSMO))
        .call(users[0])
        .unwrap();

    vault
        .bond()
        .with_funds(&coins(600, OSMO))
        .call(users[1])
        .unwrap();

    vault.stake(&contract, users[0], validators[0], coin(200, OSMO));
    vault.stake(&contract, users[0], validators[1], coin(100, OSMO));
    vault.stake(&contract, users[1], validators[0], coin(300, OSMO));

    // Start with equal distribution:
    // 20 tokens for users[0]
    // 30 tokens for users[1]
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(50, STAR))
        .call(owner)
        .unwrap();

    // Only users[0] stakes on validators[1]
    // 30 tokens for users[0]
    contract
        .test_distribute_rewards(validators[1].to_owned(), coin(30, STAR))
        .call(owner)
        .unwrap();

    // Check how much rewards are pending for withdrawal
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 20);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 30);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 30);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    // Show all rewards skips validators that were never staked on
    let all_rewards = contract
        .all_pending_rewards(users[0].to_owned(), None, None)
        .unwrap();
    let expected = vec![
        ValidatorPendingRewards::new(validators[0], 20, STAR),
        ValidatorPendingRewards::new(validators[1], 30, STAR),
    ];
    assert_eq!(all_rewards.rewards, expected);

    let all_rewards = contract
        .all_pending_rewards(users[1].to_owned(), None, None)
        .unwrap();
    let expected = vec![ValidatorPendingRewards::new(validators[0], 30, STAR)];
    assert_eq!(all_rewards.rewards, expected);

    // Some more distribution, this time not divisible by total staken tokens
    // 28 tokens for users[0]
    // 42 tokens for users[1]
    // 1 token is not distributed
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(71, STAR))
        .call(owner)
        .unwrap();

    // Distribution in invalid coin should fail
    contract
        .test_distribute_rewards(validators[1].to_owned(), coin(100, OSMO))
        .call(owner)
        .unwrap_err();

    // Check how much rewards are pending for withdrawal
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 48);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 72);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 30);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    // Withdraw rewards
    contract
        .withdraw_rewards(validators[0].to_owned(), remote[0].to_owned())
        .call(users[0])
        .unwrap();

    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_owned(), remote[0].to_owned())
        .call(users[0])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[0].to_owned(), remote[1].to_owned())
        .call(users[1])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[1])
        .unwrap();

    // Rewards withdrawal should not affect the stake
    let stake = contract
        .stake(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(200)));

    let stake = contract
        .stake(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    let stake = contract
        .stake(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(300)));

    // error if 0 rewards available
    let err = contract
        .withdraw_rewards(validators[1].to_owned(), remote[1].to_owned())
        .call(users[1])
        .unwrap_err();
    assert_eq!(err, ContractError::NoRewards);
    let tx_id = get_last_external_staking_pending_tx_id(&contract);
    assert_eq!(tx_id, None);

    // Stake remains unaffected after rewards withdrawal failure
    let stake = contract
        .stake(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(0)));

    // Rewards should not be withdrawable anymore
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    // Another distribution - making it equal
    // 4 on users[0]
    // 6 on users[1]
    //
    // The additional 1 token is leftover after previous allocation
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(9, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 4);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 6);

    // Now yet another unequal distribution to play around keeping all correct when weights are
    // changing
    //
    // 4 on users[0] (+ ~0.4)
    // 6 on users[1] (+ ~0.6)
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(11, STAR))
        .call(owner)
        .unwrap();

    // Also leaving some rewards on validator[1]
    //
    // 11 on users[0]
    contract
        .test_distribute_rewards(validators[1].to_owned(), coin(11, STAR))
        .call(owner)
        .unwrap();

    // Unstaking some funds from validator should change weights - now users split validators[0]
    // 50/50
    //
    // 200 tokens staken by user[0]
    // 200 tokens staken by user[1]
    contract
        .unstake(validators[0].to_owned(), coin(100, OSMO))
        .call(users[1])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Staking also changes weights - now validators[1] also splits rewards:
    // 1/4 for users[0]
    // 3/4 for users[1]
    //
    // 100 tokens staken by user[0]
    // 300 tokens staken by user[1]
    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(300, OSMO),
            to_json_binary(&ReceiveVirtualStake {
                validator: validators[1].to_string(),
            })
            .unwrap(),
        )
        .call(users[1])
        .unwrap();
    contract
        .test_commit_stake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Check if messing up with weights didn't affect withdrawable
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 8);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 12);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 11);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    // Now distribute some nice values
    // 10 on users[0] (~0.4 still not distributed)
    // 10 on users[1] (~0.6 still not distributed)
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(20, STAR))
        .call(owner)
        .unwrap();

    // Also for validator[1]
    // 10 on users[1]
    // 30 on users[2]
    contract
        .test_distribute_rewards(validators[1].to_owned(), coin(40, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 18);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 22);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 21);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 30);

    // And some more distribution fun - we are 50/50 on validators[1], so distributing odd number of
    // coins
    // 2 for users[0] (+ ~0.5 from this distribution + ~0.4 accumulated -> ~0.9 tokens should be
    //   here)
    // 3 for users[1] (+ ~0.5 from this distribution + ~0.6 accumulated -> ~1.1 tokens, we give one
    //   back leaving ~0.1 accumulated)
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(5, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 20);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 25);

    // More unstaking - to make it both ways by both stakers on at least one validator, for sake of
    // funny error accumulation issues. After two following unstakes, staking on validators[0] is as
    // follows:
    //
    // 150 tokens staken by users[0]
    // 100 tokens staken by users[1]
    //
    // Rewards distribution:
    //
    // 3/5 rewards to users[0]
    // 2/5 rewards to users[1]
    contract
        .unstake(validators[0].to_owned(), coin(50, OSMO))
        .call(users[0])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    contract
        .unstake(validators[0].to_owned(), coin(100, OSMO))
        .call(users[1])
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Distribute 12 tokens to validator[0]:
    //
    // 7 + 1 = 8 to users[0] (~0.9 accumulated + ~0.2 = ~1.1 leftover, 1.0 payed back, ~0.1 accumulated)
    // 4 to users[0] (~0.1 accumulated + ~0.8 -> leaving at ~0.9)
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(12, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 28);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 29);

    // Withdraw only by users[0]
    contract
        .withdraw_rewards(validators[0].to_owned(), remote[0].to_owned())
        .call(users[0])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_owned(), remote[0].to_owned())
        .call(users[0])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    // Rollback on users[1]
    contract
        .withdraw_rewards(validators[0].to_owned(), "bad_value".to_owned())
        .call(users[1])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_rollback_withdraw_rewards(tx_id)
        .call(users[1])
        .unwrap();

    // Rewards withdrawal should not affect the stake
    let stake = contract
        .stake(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(150)));

    let stake = contract
        .stake(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    let stake = contract
        .stake(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    let stake = contract
        .stake(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(300)));

    // Check withdrawals and accounts
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 29);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 30);

    // Final distribution - 10 tokens to both validators
    // 6 tokens to users[0] via validators[0] (leftover as it was)
    // 4 tokens to users[1] via validators[0] (leftover as it was)
    // 2 tokens to users[0] via validators[1] (~0.5 leftover)
    // 7 tokens to users[1] via validators[1] (~0.5 lefover)
    contract
        .test_distribute_rewards(validators[0].to_owned(), coin(10, STAR))
        .call(owner)
        .unwrap();

    contract
        .test_distribute_rewards(validators[1].to_owned(), coin(10, STAR))
        .call(owner)
        .unwrap();

    // Check withdrawals and accounts
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 6);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 33);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 2);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap()
        .rewards;
    assert_eq!(rewards.amount.u128(), 37);

    let all_rewards = contract
        .all_pending_rewards(users[0].to_owned(), None, None)
        .unwrap();
    let expected = vec![
        ValidatorPendingRewards::new(validators[0], 6, STAR),
        ValidatorPendingRewards::new(validators[1], 2, STAR),
    ];
    assert_eq!(all_rewards.rewards, expected);

    let all_rewards = contract
        .all_pending_rewards(users[1].to_owned(), None, None)
        .unwrap();
    let expected = vec![
        ValidatorPendingRewards::new(validators[0], 33, STAR),
        ValidatorPendingRewards::new(validators[1], 37, STAR),
    ];
    assert_eq!(all_rewards.rewards, expected);

    // And try to withdraw all, previous balances:
    contract
        .withdraw_rewards(validators[0].to_string(), remote[0].to_owned())
        .call(users[0])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_string(), remote[0].to_owned())
        .call(users[0])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[0].to_string(), remote[1].to_owned())
        .call(users[1])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_string(), remote[1].to_owned())
        .call(users[1])
        .unwrap();
    let tx_id = get_last_external_staking_pending_tx_id(&contract).unwrap();
    contract
        .test_commit_withdraw_rewards(tx_id)
        .call(users[0])
        .unwrap();
}

#[test]
fn batch_distribution() {
    let owner = "owner";
    let users = ["user1", "user2"];

    let app =
        App::new_with_balances(&[(users[0], &coins(600, OSMO)), (users[1], &coins(600, OSMO))]);

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(600, OSMO))
        .call(users[0])
        .unwrap();
    vault
        .bond()
        .with_funds(&coins(600, OSMO))
        .call(users[1])
        .unwrap();

    vault.stake(&contract, users[0], validators[0], coin(200, OSMO));
    vault.stake(&contract, users[0], validators[1], coin(100, OSMO));
    vault.stake(&contract, users[1], validators[0], coin(300, OSMO));

    contract
        .distribute_batch(owner, STAR, &[(validators[0], 50), (validators[1], 30)])
        .unwrap();

    assert_rewards!(contract, users[0], validators[0], 20);
    assert_rewards!(contract, users[1], validators[0], 30);
    assert_rewards!(contract, users[0], validators[1], 30);
    assert_rewards!(contract, users[1], validators[1], 0);

    contract
        .distribute_batch(owner, STAR, &[(validators[0], 100), (validators[1], 30)])
        .unwrap();

    assert_rewards!(contract, users[0], validators[0], 60);
    assert_rewards!(contract, users[1], validators[0], 90);
    assert_rewards!(contract, users[0], validators[1], 60);
    assert_rewards!(contract, users[1], validators[1], 0);
}

#[test]
fn batch_distribution_invalid_token() {
    let owner = "owner";
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(600, OSMO))]);

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validator = contract.activate_validators(["validator1"])[0];

    vault
        .bond()
        .with_funds(&coins(600, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validator, coin(200, OSMO));

    let err = contract
        .distribute_batch(owner, "supertoken", &[(validator, 50)])
        .unwrap_err();
    assert_eq!(err, ContractError::InvalidDenom(STAR.to_string()));
}

#[test]
fn slashing() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(300, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validators[0], coin(200, OSMO));
    vault.stake(&contract, user, validators[1], coin(100, OSMO));

    // Unstake some tokens
    // user unstakes 50 from validators[0] - 150 left staked in 2 batches
    contract
        .unstake(validators[0].to_string(), coin(50, OSMO))
        .call(user)
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Unstaken should be immediately visible on staked amount
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(150)));

    let stake = contract
        .stake(user.to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    // But not on vault side
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    // Very short travel in future - too short for claims to release
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(50);
    });

    // Withdrawing liens
    contract.withdraw_unbonded().call(user).unwrap();

    // Claims still not changed on the vault side - withdrawal to early
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 300);

    // Adding some more unstakes
    // user unstakes 70 from validators[0] - 80 left staken
    contract
        .unstake(validators[0].to_owned(), coin(70, OSMO))
        .call(user)
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    contract
        .unstake(validators[1].to_owned(), coin(90, OSMO))
        .call(user)
        .unwrap();
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Verify proper stake values
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(80)));

    let stake = contract
        .stake(user.to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(10)));

    // Another timetravel - just enough for first batch of stakes to release,
    // too early for second batch
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(50);
    });

    // But now validators[0] slashing happens
    contract
        .test_handle_slashing(validators[0].to_string(), Uint128::new(8))
        .call("test")
        .unwrap();

    // Claims on vault got reduced, but only for bonded and second batch amount slashing (10% of (80 + 70) = 15)
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 285);

    // Withdrawing liens
    contract.withdraw_unbonded().call(user).unwrap();

    // Now claims on vault got reduced, but only for first batch amount (not slashed)
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 235);

    // Moving forward more, passing through second bath pending duration
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(60);
    });

    // Nothing gets released automatically, values just like before
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 235);

    // Withdrawing liens
    contract.withdraw_unbonded().call(user).unwrap();

    // Now everything is released (235 - 90 - 70 + 7 = 82)
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 82);
}

#[test]
fn slashing_pending_tx_partial_unbond() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(300, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validators[0], coin(200, OSMO));
    vault.stake(&contract, user, validators[1], coin(100, OSMO));

    // Unstake some tokens
    contract
        .unstake(validators[0].to_string(), coin(50, OSMO))
        .call(user)
        .unwrap();

    // Unstaken should be immediately visible on staked amount (as pending tx)
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(
        stake.stake,
        ValueRange::new(Uint128::new(150), Uint128::new(200))
    );

    let stake = contract
        .stake(user.to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(100)));

    // Now validators[0] slashing happens
    contract
        .test_handle_slashing(validators[0].to_string(), Uint128::new(20))
        .call("test")
        .unwrap();

    // Claims on vault got reduced, for high end of pending unbond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 280);

    // Now the unbond gets committed (i.e. successful)
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 280);

    // Time travel - just enough for unbond to release,
    app.app_mut().update_block(|block| {
        block.height += 2;
        block.time = block.time.plus_seconds(100);
    });

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 280);

    // Withdrawing liens
    contract.withdraw_unbonded().call(user).unwrap();

    // Now claims on vault got reduced by the (full) unbonded amount
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 230);
}

#[test]
fn slashing_pending_tx_full_unbond() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(200, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validators[0], coin(200, OSMO));

    // Unstake all tokens
    contract
        .unstake(validators[0].to_string(), coin(200, OSMO))
        .call(user)
        .unwrap();

    // Unstaken should be immediately visible on staked amount (as pending tx)
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(
        stake.stake,
        ValueRange::new(Uint128::new(0), Uint128::new(200))
    );

    // Now validators[0] slashing happens
    contract
        .test_handle_slashing(validators[0].to_string(), Uint128::new(20))
        .call("test")
        .unwrap();

    // Claims on vault got reduced, for high end of pending unbond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);

    // Now the unbond gets committed (i.e. successful)
    contract
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);

    // Time travel - just enough for unbond to release,
    app.app_mut().update_block(|block| {
        block.height += 2;
        block.time = block.time.plus_seconds(100);
    });

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);

    // Withdrawing liens
    contract.withdraw_unbonded().call(user).unwrap();

    // Now claims on vault got reduced by the (full) unbonded amount
    let err = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap_err();
    assert!(err
        .to_string()
        .contains(&mesh_vault::error::ContractError::NoClaim.to_string()));
}

#[test]
fn slashing_pending_tx_full_unbond_rolled_back() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(200, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1"]);

    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validators[0], coin(200, OSMO));

    // Unstake all tokens
    contract
        .unstake(validators[0].to_string(), coin(200, OSMO))
        .call(user)
        .unwrap();

    // Unstaken should be immediately visible on staked amount (as pending tx)
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(
        stake.stake,
        ValueRange::new(Uint128::new(0), Uint128::new(200))
    );

    // Now validators[0] slashing happens
    contract
        .test_handle_slashing(validators[0].to_string(), Uint128::new(20))
        .call("test")
        .unwrap();

    // Claims on vault got reduced, for high end of pending unbond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);

    // Now the unbond gets rolled back (i.e. failed)
    contract
        .test_rollback_unstake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);

    // Time travel - just enough for unbond to release,
    app.app_mut().update_block(|block| {
        block.height += 2;
        block.time = block.time.plus_seconds(100);
    });

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);

    // Withdrawing liens
    contract.withdraw_unbonded().call(user).unwrap();

    // Claims on vault are still unchanged
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 180);
}

#[test]
fn slashing_pending_tx_bond() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(300, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validators[0], coin(200, OSMO));
    vault.stake(&contract, user, validators[1], coin(50, OSMO));

    // Stake some more tokens (but don't commit them!)
    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(50, OSMO),
            to_json_binary(&ReceiveVirtualStake {
                validator: validators[0].into(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // Staken should be immediately visible on staked amount (as pending tx)
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(
        stake.stake,
        ValueRange::new(Uint128::new(200), Uint128::new(250))
    );

    let stake = contract
        .stake(user.to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(50)));

    // Claims on vault got adjusted, to account for pending bond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(
        claim.amount,
        ValueRange::new(Uint128::new(250), Uint128::new(300))
    );

    // Now validators[0] slashing happens, over the amount included the pending bond
    contract
        .test_handle_slashing(validators[0].to_string(), Uint128::new(25))
        .call("test")
        .unwrap();

    // Claims on vault got reduced, for high end of pending slashed bond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(
        claim.amount,
        ValueRange::new(Uint128::new(225), Uint128::new(275))
    );

    // Now the extra bond gets committed (i.e. successful)
    contract
        .test_commit_stake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Claims on vault are now committed
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 275);
}

#[test]
fn slashing_pending_tx_bond_rolled_back() {
    let user = "user1";

    let app = App::new_with_balances(&[(user, &coins(300, OSMO))]);

    let owner = "owner";

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    let validators = contract.activate_validators(["validator1", "validator2"]);

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    vault.stake(&contract, user, validators[0], coin(200, OSMO));
    vault.stake(&contract, user, validators[1], coin(50, OSMO));

    // Stake some more tokens (but don't commit them!)
    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(50, OSMO),
            to_json_binary(&ReceiveVirtualStake {
                validator: validators[0].into(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // Staken should be immediately visible on staked amount (as pending tx)
    let stake = contract
        .stake(user.to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(
        stake.stake,
        ValueRange::new(Uint128::new(200), Uint128::new(250))
    );

    let stake = contract
        .stake(user.to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake, ValueRange::new_val(Uint128::new(50)));

    // Claims on vault got adjusted, to account for pending bond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(
        claim.amount,
        ValueRange::new(Uint128::new(250), Uint128::new(300))
    );

    // Now validators[0] slashing happens, but over the amount without the pending bond
    contract
        .test_handle_slashing(validators[0].to_string(), Uint128::new(20))
        .call("test")
        .unwrap();

    // Claims on vault got reduced, for *low* end of pending slashed bond
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(
        claim.amount,
        ValueRange::new(Uint128::new(230), Uint128::new(280))
    );

    // Now the extra bond gets rolled back (i.e. failed)
    contract
        .test_rollback_stake(get_last_external_staking_pending_tx_id(&contract).unwrap())
        .call("test")
        .unwrap();

    // Claims on vault are now committed
    let claim = vault
        .claim(user.to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.val().unwrap().u128(), 230);
}
