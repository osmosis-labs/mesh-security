use cosmwasm_schema::cw_serde;
use cosmwasm_std::{BlockInfo, Timestamp, Uint128};
use mesh_apis::vault_api::VaultApiHelper;

/// Contract configuration
#[cw_serde]
pub struct Config {
    /// Local native token this contracts operate on
    pub denom: String,
    /// Vault contract address
    pub vault: VaultApiHelper,
    /// Ubbounding period for claims in seconds
    pub unbonding_period: u64,
}

/// All user/account related information
#[cw_serde]
#[derive(Default)]
pub struct User {
    /// How much tokens user staken and not in unbonding period
    /// via this contract
    pub stake: Uint128,
    /// List of token batches scheduled for unboding
    ///
    /// Items should only be added to the end of this list, with `release_at` being
    /// `unbonding_period` after current time - this way this is guaranteed to be
    /// always sorted (as time is guaranteed to be monotonic).
    pub pending_unbonds: Vec<PendingUnbond>,
}

/// Description of tokens in unbonding period
#[cw_serde]
pub struct PendingUnbond {
    /// Tokens scheduled for unbonding
    pub amount: Uint128,
    /// Time when tokens are released
    pub release_at: Timestamp,
}

impl User {
    /// Removes expired entries from `pending_unbonds`, returning amount of tokens released.
    pub fn release_pending(&mut self, info: &BlockInfo) -> Uint128 {
        // The fact that `pending unbonds are always added to the end, so they are always ordered
        // is assumed here.

        // Nothing waits for unbond
        if self.pending_unbonds.is_empty() {
            return Uint128::zero();
        };

        // First item is still not ready for release
        if self.pending_unbonds[0].release_at < info.time {
            return Uint128::zero();
        }

        let non_expired_idx = self
            .pending_unbonds
            .partition_point(|pending| pending.release_at >= info.time);

        self.pending_unbonds
            .drain(..non_expired_idx)
            .map(|pending| pending.amount)
            .sum()
    }
}
