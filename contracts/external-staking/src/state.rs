use cosmwasm_schema::cw_serde;
use cosmwasm_std::{BlockInfo, Timestamp, Uint128, Uint256};
use mesh_apis::vault_api::VaultApiHelper;

/// Contract configuration
#[cw_serde]
pub struct Config {
    /// Local native token this contracts operate on
    pub denom: String,
    /// Rewards token for this contract (remote IBC token)
    pub rewards_denom: String,
    /// Vault contract address
    pub vault: VaultApiHelper,
    /// Ubbounding period for claims in seconds
    pub unbonding_period: u64,
}

/// All single stake related information - entry per `(user, validator)` pair, including
/// distribution alignment
#[cw_serde]
pub struct Stake {
    /// How much tokens user staken and not in unbonding period
    /// via this contract
    pub stake: Uint128,
    /// List of token batches scheduled for unbonding
    ///
    /// Items should only be added to the end of this list, with `release_at` being
    /// `unbonding_period` after current time - this way this is guaranteed to be
    /// always sorted (as time is guaranteed to be monotonic).
    pub pending_unbonds: Vec<PendingUnbond>,
    /// Points alignment is how much points should be added/substracted from points caltulated per
    /// user due to stake changes. It has to be signed type, but no signed integrals are right now
    /// in CosmWasm - using `Uint256` here as a "fake" type, so for calculations it is shifted - the
    /// real value storedis `points_alignment - Uint256::MAX / 2` - this is not ideal, but it makes
    /// calculations always fit in U256.
    pub points_alignment: Uint256,
    /// Tokens already withdrawn by this user
    pub withdrawn_funds: Uint128,
}

impl Default for Stake {
    fn default() -> Self {
        Self {
            stake: Default::default(),
            pending_unbonds: Default::default(),
            // We want this value to be shifted by `U256::MAX` for keeping calculations in
            // reasonable range
            points_alignment: Uint256::MAX / Uint256::from_u128(2),
            withdrawn_funds: Default::default(),
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
