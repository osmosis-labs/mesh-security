use anyhow::Result as AnyResult;

use cosmwasm_std::{coin, coins, to_binary, Addr, Decimal};
use mesh_native_staking::contract::multitest_utils::CodeId as NativeStakingCodeId;
use mesh_native_staking::contract::InstantiateMsg as NativeStakingInstantiateMsg;
use mesh_native_staking_proxy::contract::multitest_utils::CodeId as NativeStakingProxyCodeId;
use mesh_vault::contract::multitest_utils::{CodeId as VaultCodeId, VaultContractProxy};
use mesh_vault::contract::test_utils::VaultApi;
use mesh_vault::msg::StakingInitInfo;

use mesh_sync::Tx;

use cw_multi_test::App as MtApp;
use sylvia::multitest::App;

use crate::contract::cross_staking::test_utils::CrossStakingApi;
use crate::contract::multitest_utils::{CodeId, ExternalStakingContractProxy};
use crate::error::ContractError;
use crate::msg::{ReceiveVirtualStake, StakeInfo};

const OSMO: &str = "osmo";
const STAR: &str = "star";

// Shortcut setuping all needed contracts
//
// Returns vault and external staking proxies
fn setup<'app>(
    app: &'app App,
    owner: &str,
    unbond_period: u64,
) -> AnyResult<(VaultContractProxy<'app>, ExternalStakingContractProxy<'app>)> {
    let native_staking_proxy_code = NativeStakingProxyCodeId::store_code(app);
    let native_staking_code = NativeStakingCodeId::store_code(app);
    let vault_code = VaultCodeId::store_code(app);
    let contract_code = CodeId::store_code(app);

    let native_staking_instantiate = NativeStakingInstantiateMsg {
        denom: OSMO.to_owned(),
        proxy_code_id: native_staking_proxy_code.code_id(),
    };

    let staking_init = StakingInitInfo {
        admin: None,
        code_id: native_staking_code.code_id(),
        msg: to_binary(&native_staking_instantiate)?,
        label: Some("Native staking".to_owned()),
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init)
        .call(owner)?;

    let contract = contract_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
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

    let max_slash = contract.cross_staking_api_proxy().max_slash().unwrap();
    assert_eq!(max_slash.max_slash, Decimal::percent(5));
}

#[test]
fn staking() {
    let users = ["user1", "user2"];

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[0]), coins(300, OSMO))
            .unwrap();

        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[1]), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    let owner = "owner";
    let validators = ["validator1", "validator2"];

    let (vault, contract) = setup(&app, owner, 100).unwrap();

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

    // Perform couple stakes
    // users[0] stakes 200 on validators[0] in 2 batches
    // users[0] stakes 100 on validators[1]
    // users[1] stakes 100 on validators[0]
    // users[1] stakes 200 on validators[1]
    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();

    let last_tx = get_last_pending_tx_id(&vault).unwrap();
    // Hardcoded commit_tx call (lack of IBC support yet)
    vault
        .vault_api_proxy()
        .commit_tx(last_tx)
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[1].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();

    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();

    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(200, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[1].to_string(),
            })
            .unwrap(),
        )
        .call(users[1])
        .unwrap();

    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[1])
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

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
    assert_eq!(stake.stake.u128(), 200);

    let stake = contract
        .stake(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake.u128(), 100);

    let stake = contract
        .stake(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(stake.stake.u128(), 100);

    let stake = contract
        .stake(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(stake.stake.u128(), 200);

    // Querying fo all the stakes
    let stakes = contract.stakes(users[0].to_owned(), None, None).unwrap();
    assert_eq!(
        stakes.stakes,
        [
            StakeInfo {
                owner: users[0].to_owned(),
                validator: validators[0].to_owned(),
                stake: 200u128.into()
            },
            StakeInfo {
                owner: users[0].to_owned(),
                validator: validators[1].to_owned(),
                stake: 100u128.into()
            },
        ]
    );

    let stakes = contract.stakes(users[1].to_owned(), None, None).unwrap();
    assert_eq!(
        stakes.stakes,
        [
            StakeInfo {
                owner: users[1].to_owned(),
                validator: validators[0].to_owned(),
                stake: 100u128.into()
            },
            StakeInfo {
                owner: users[1].to_owned(),
                validator: validators[1].to_owned(),
                stake: 200u128.into()
            },
        ]
    );
}

#[track_caller]
fn get_last_pending_tx_id(vault: &VaultContractProxy) -> Option<u64> {
    let txs = vault.all_pending_txs_desc(None, None).unwrap().txs;
    txs.first().map(|tx| match tx {
        Tx::InFlightStaking { id, .. } => *id,
    })
}

#[test]
fn unstaking() {
    let users = ["user1", "user2"];

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[0]), coins(300, OSMO))
            .unwrap();

        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[1]), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    let owner = "owner";
    let validators = ["validator1", "validator2"];

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    // Bond and stake tokens
    //
    // users[0] stakes 200 on validators[0]
    // users[0] stakes 100 on validators[1]
    // users[1] stakes 300 on validators[0]
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

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(200, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();
    let last_tx = get_last_pending_tx_id(&vault).unwrap();
    // Hardcoded commit_tx call (lack of IBC support yet)
    vault
        .vault_api_proxy()
        .commit_tx(last_tx)
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[1].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(300, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[1])
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    // Properly unstake some tokens
    // users[0] unstakes 50 from validators[0] - 150 left staken in 2 batches
    // users[1] usntakes 60 from validators[0] - 240 left staken
    contract
        .unstake(validators[0].to_string(), coin(20, OSMO))
        .call(users[0])
        .unwrap();

    contract
        .unstake(validators[0].to_string(), coin(30, OSMO))
        .call(users[0])
        .unwrap();

    contract
        .unstake(validators[0].to_string(), coin(60, OSMO))
        .call(users[1])
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
    assert_eq!(stake.stake.u128(), 150);

    let stake = contract
        .stake(users[0].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 100);

    let stake = contract
        .stake(users[1].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 240);

    let stake = contract
        .stake(users[1].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 0);

    // But not on vault side
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 300);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 300);

    // Immediately withdrawing liens
    contract.withdraw_unbonded().call(users[0]).unwrap();
    contract.withdraw_unbonded().call(users[1]).unwrap();

    // Claims still not changed on the vault side
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 300);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 300);

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
    assert_eq!(claim.amount.u128(), 300);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 300);

    // Adding some more unstakes
    // users[0] unstakes 70 from validators[0] - 80 left staken
    // users[1] unstakes 90 from validators[1] = 10 left staken
    contract
        .unstake(validators[0].to_owned(), coin(70, OSMO))
        .call(users[0])
        .unwrap();

    contract
        .unstake(validators[1].to_owned(), coin(90, OSMO))
        .call(users[0])
        .unwrap();

    // Verify proper stake values
    let stake = contract
        .stake(users[0].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 80);

    let stake = contract
        .stake(users[0].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 10);

    let stake = contract
        .stake(users[1].to_string(), validators[0].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 240);

    let stake = contract
        .stake(users[1].to_string(), validators[1].to_string())
        .unwrap();
    assert_eq!(stake.stake.u128(), 0);

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
    assert_eq!(claim.amount.u128(), 250);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 240);

    // Moving forward more, passing through second bath pending duration
    app.app_mut().update_block(|block| {
        block.height += 1;
        block.time = block.time.plus_seconds(60);
    });

    // Nothing gets released automatically, values just like before
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 250);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 240);

    // Withdrawing liens
    contract.withdraw_unbonded().call(users[0]).unwrap();
    contract.withdraw_unbonded().call(users[1]).unwrap();

    // Now everything is released
    let claim = vault
        .claim(users[0].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 90);

    let claim = vault
        .claim(users[1].to_owned(), contract.contract_addr.to_string())
        .unwrap();
    assert_eq!(claim.amount.u128(), 240);
}

#[test]
fn distribution() {
    let owner = "owner";
    let users = ["user1", "user2"];

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[0]), coins(600, OSMO))
            .unwrap();

        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[1]), coins(600, OSMO))
            .unwrap();

        router
            .bank
            .init_balance(
                storage,
                &Addr::unchecked(owner),
                vec![coin(1000, STAR), coin(1000, OSMO)],
            )
            .unwrap();
    });
    let app = App::new(app);

    let validators = ["validator1", "validator2"];

    let (vault, contract) = setup(&app, owner, 100).unwrap();

    // Bond and stake tokens
    //
    // users[0] stakes 200 on validators[0]
    // users[0] stakes 100 on validators[1]
    // users[1] stakes 300 on validators[0]
    //
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

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(200, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();
    // Hardcoded commit_tx call (lack of IBC support yet)
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[1].to_string(),
            })
            .unwrap(),
        )
        .call(users[0])
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            contract.contract_addr.to_string(),
            coin(300, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validators[0].to_string(),
            })
            .unwrap(),
        )
        .call(users[1])
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(contract.contract_addr.as_str())
        .unwrap();

    // Start with equal distribution:
    // 20 tokens for users[0]
    // 30 tokens for users[1]
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(50, STAR))
        .call(owner)
        .unwrap();

    // Only users[0] stakes on validators[1]
    // 30 tokens for users[1]
    contract
        .distribute_rewards(validators[1].to_owned())
        .with_funds(&coins(30, STAR))
        .call(owner)
        .unwrap();

    // Check how much rewards are pending for withdrawal
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 20);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 30);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 30);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    // Distributed funds should be on the staking contract
    assert_eq!(
        app.app()
            .wrap()
            .query_all_balances(contract.contract_addr.clone())
            .unwrap(),
        coins(80, STAR)
    );

    // Some more distribution, this time not divisible by total staken tokens
    // 28 tokens for users[0]
    // 42 tokens for users[1]
    // 1 token is not distributed
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(71, STAR))
        .call(owner)
        .unwrap();

    // Distribution in invalid coin should fail
    contract
        .distribute_rewards(validators[1].to_owned())
        .with_funds(&coins(100, OSMO))
        .call(owner)
        .unwrap_err();

    // Check how much rewards are pending for withdrawal
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 48);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 72);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 30);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    // Withdraw rewards
    contract
        .withdraw_rewards(validators[0].to_owned())
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_owned())
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[0].to_owned())
        .call(users[1])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_owned())
        .call(users[1])
        .unwrap();

    // Rewards should not be withdrawable anymore
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    // Rewads should be on users accounts
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(users[0], STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        78
    );

    assert_eq!(
        app.app()
            .wrap()
            .query_balance(users[1], STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        72
    );

    // Anothed distribution - making it equal
    // 4 on users[0]
    // 6 on users[1]
    //
    // The additional 1 token is leftover after previous allocation
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(9, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 4);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 6);

    // Now yet another unequal distribution to play around keeping all correct when weights are
    // changing
    //
    // 4 on users[0] (+ ~0.4)
    // 6 on users[1] (+ ~0.6)
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(11, STAR))
        .call(owner)
        .unwrap();

    // Also leaving some rewards on validator[1]
    //
    // 11 on users[0]
    contract
        .distribute_rewards(validators[1].to_owned())
        .with_funds(&coins(11, STAR))
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
            to_binary(&ReceiveVirtualStake {
                validator: validators[1].to_string(),
            })
            .unwrap(),
        )
        .call(users[1])
        .unwrap();

    // Check if messing up with weights didn't affect withdrawable
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 8);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 12);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 11);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    // Now distribute some nice values
    // 10 on users[0] (~0.4 still not distributed)
    // 10 on users[1] (~0.6 still not distributed)
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(20, STAR))
        .call(owner)
        .unwrap();

    // Also for validator[1]
    // 10 on users[1]
    // 30 on users[2]
    contract
        .distribute_rewards(validators[1].to_owned())
        .with_funds(&coins(40, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 18);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 22);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 21);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 30);

    // And some more distribution fun - we are 50/50 on validators[1], so distributing odd number of
    // coins
    // 2 for users[0] (+ ~0.5 from this distribution + ~0.4 accumulated -> ~0.9 tokens should be
    //   here)
    // 3 for users[1] (+ ~0.5 from this distribution + ~0.6 accumulated -> ~1.1 tokens, we give one
    //   back leaving ~0.1 accumulated)
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(5, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 20);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 25);

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
        .unstake(validators[0].to_owned(), coin(100, OSMO))
        .call(users[1])
        .unwrap();

    // Distribute 12 tokens to validator[0]:
    //
    // 7 + 1 = 8 to users[0] (~0.9 accumulated + ~0.2 = ~1.1 leftover, 1.0 payed back, ~0.1 accumulated)
    // 4 to users[0] (~0.1 accumulated + ~0.8 -> leaving at ~0.9)
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(12, STAR))
        .call(owner)
        .unwrap();

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 28);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 29);

    // Withdraw only by users[0]
    contract
        .withdraw_rewards(validators[0].to_owned())
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_owned())
        .call(users[0])
        .unwrap();

    // Check withdrawals and accounts
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 29);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 0);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 30);

    // Balances was previously:
    // 78 on users[0] - now witdrawing 28 from validators[0] and 21 from validators[1]
    // 72 on users[1] - should be the same
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(users[0], STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        127
    );

    assert_eq!(
        app.app()
            .wrap()
            .query_balance(users[1], STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        72
    );

    // On contract we keep 59 to be withdrawn by users[1] + 1 token of leftover
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(contract.contract_addr.to_string(), STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        60
    );

    // Final distribution - 10 tokens to both validators
    // 6 tokens to users[0] via validators[0] (leftover as it was)
    // 4 tokens to users[1] via validators[0] (leftover as it was)
    // 2 tokens to users[0] via validators[1] (~0.5 leftover)
    // 7 tokens to users[1] via validators[1] (~0.5 lefover)
    contract
        .distribute_rewards(validators[0].to_owned())
        .with_funds(&coins(10, STAR))
        .call(owner)
        .unwrap();

    contract
        .distribute_rewards(validators[1].to_owned())
        .with_funds(&coins(10, STAR))
        .call(owner)
        .unwrap();

    // Check withdrawals and accounts
    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 6);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[0].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 33);

    let rewards = contract
        .pending_rewards(users[0].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 2);

    let rewards = contract
        .pending_rewards(users[1].to_owned(), validators[1].to_owned())
        .unwrap();
    assert_eq!(rewards.amount.amount.u128(), 37);

    // And try to withdraw all, previous balances:
    contract
        .withdraw_rewards(validators[0].to_string())
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_string())
        .call(users[0])
        .unwrap();

    contract
        .withdraw_rewards(validators[0].to_string())
        .call(users[1])
        .unwrap();

    contract
        .withdraw_rewards(validators[1].to_string())
        .call(users[1])
        .unwrap();

    // Varyfying accounts, previous states:
    // 127 on users[0] - now withdrawn 6 from validators[0] and 2 from validators[1]
    // 72 on users[1] - now withdrawn 33 from validators[0] and 37 from validators[1]
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(users[0], STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        135
    );

    assert_eq!(
        app.app()
            .wrap()
            .query_balance(users[1], STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        142
    );

    // Both distributions have leftover - 2 tokens accumulated on staking contract
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(contract.contract_addr.to_string(), STAR.to_owned())
            .unwrap()
            .amount
            .u128(),
        2
    );
}
