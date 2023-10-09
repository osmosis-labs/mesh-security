use cosmwasm_schema::cw_serde;
use cosmwasm_std::{BlockInfo, Decimal, Timestamp, Uint128, Uint256};
use mesh_apis::vault_api::VaultApiHelper;
use mesh_sync::ValueRange;

use crate::points_alignment::PointsAlignment;

/// Contract configuration
#[cw_serde]
pub struct Config {
    /// Local native token this contracts operate on
    pub denom: String,
    /// Rewards token for this contract (remote IBC token)
    pub rewards_denom: String,
    /// Vault contract address
    pub vault: VaultApiHelper,
    /// Unbonding period for claims in seconds
    pub unbonding_period: u64,
    /// Max slash percentage (from InstantiateMsg, maybe later from the chain)
    pub max_slashing: Decimal,
}

/// All single stake related information - entry per `(user, validator)` pair, including
/// distribution alignment
#[cw_serde]
#[derive(Default)]
pub struct Stake {
    /// How much tokens user stake and not in unbonding period
    /// via this contract
    pub stake: ValueRange<Uint128>,
    /// List of token batches scheduled for unbonding
    ///
    /// Items should only be added to the end of this list, with `release_at` being
    /// `unbonding_period` after current time - this way this is guaranteed to be
    /// always sorted (as time is guaranteed to be monotonic).
    pub pending_unbonds: Vec<PendingUnbond>,
    /// Points alignment is how much points should be added/subtracted from points calculated per
    /// user due to stake changes.
    pub points_alignment: PointsAlignment,
    /// Tokens already withdrawn by this user
    pub withdrawn_funds: Uint128,
}

impl Stake {
    /// Create simplified stake (mostly for tests)
    pub fn from_amount(amount: Uint128) -> Self {
        Self {
            stake: ValueRange::new_val(amount),
            ..Default::default()
        }
    }
}

/// Description of tokens in unbonding period
#[cw_serde]
pub struct PendingUnbond {
    /// Tokens scheduled for unbonding
    pub amount: Uint128,
    /// Time when tokens are released
    pub release_at: Timestamp,
}

impl Stake {
    /// Removes expired entries from `pending_unbonds`, returning amount of tokens released.
    pub fn release_pending(&mut self, info: &BlockInfo) -> Uint128 {
        // The fact that `pending unbonds are always added to the end, so they are always ordered
        // is assumed here.

        // Nothing waits for unbond
        if self.pending_unbonds.is_empty() {
            return Uint128::zero();
        };

        // First item is still not ready for release
        if self.pending_unbonds[0].release_at > info.time {
            return Uint128::zero();
        }

        let non_expired_idx = self
            .pending_unbonds
            .partition_point(|pending| pending.release_at <= info.time);

        self.pending_unbonds
            .drain(..non_expired_idx)
            .map(|pending| pending.amount)
            .sum()
    }

    /// Slashes all the entries in `pending_unbonds`, returning total slashed amount.
    pub fn slash_pending(&mut self, info: &BlockInfo, slash_ratio: Decimal) -> Uint128 {
        self.pending_unbonds
            .iter_mut()
            .filter(|pending| pending.release_at > info.time)
            .map(|pending| {
                let slash = pending.amount * slash_ratio;
                // Slash it
                pending.amount -= slash;
                slash
            })
            .sum()
    }
}

/// Per validator distribution information
#[cw_serde]
#[derive(Default)]
pub struct Distribution {
    /// Total tokens staken on this validator by all users
    pub total_stake: Uint128,
    /// Points user is eligible to by single token staken
    pub points_per_stake: Uint256,
    /// Points which were not distributed previously
    pub points_leftover: Uint256,
}
