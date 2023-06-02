use cosmwasm_std::{coin, coins, to_binary, Addr, Decimal};
use mesh_native_staking::contract::multitest_utils::CodeId as NativeStakingCodeId;
use mesh_native_staking::contract::InstantiateMsg as NativeStakingInstantiateMsg;
use mesh_native_staking_proxy::contract::multitest_utils::CodeId as NativeStakingProxyCodeId;
use mesh_vault::contract::multitest_utils::{CodeId as VaultCodeId, VaultContractProxy};
use mesh_vault::msg::StakingInitInfo;

use cw_multi_test::App as MtApp;
use sylvia::multitest::App;

use crate::contract::cross_staking::test_utils::CrossStakingApi;
use crate::contract::multitest_utils::{CodeId, ExternalStakingContractProxy};
use crate::error::ContractError;
use crate::msg::{ReceiveVirtualStake, StakeInfo};

use anyhow::Result as AnyResult;

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
