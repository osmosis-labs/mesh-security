use cosmwasm_schema::cw_serde;
use cosmwasm_std::{BlockInfo, Uint128};
use cw_utils::{Duration, Expiration};
use mesh_apis::vault_api::VaultApiHelper;

/// Contract configuration
#[cw_serde]
pub struct Config {
    /// Local native token this contracts operate on
    pub denom: String,
    /// Vault contract address
    pub vault: VaultApiHelper,
    /// Ubbounding period for claims
    pub unbounding_period: Duration,
}

/// All user/account related information
#[cw_serde]
#[derive(Default)]
pub struct User {
    /// How much tokens user staken and not in unbounding period
    /// via this contract
    pub stake: Uint128,
    /// List of token batches scheduled for unbouding
    pub pending_unbounds: Vec<PendingUnbound>,
    /// Tokens already released, but not yet claimed
    pub released: Uint128,
}

/// Description of tokens in unbounding period
#[cw_serde]
pub struct PendingUnbound {
    /// Tokens scheduled for unbounding
    pub amount: Uint128,
    /// Time when tokens are released
    pub release_at: Expiration,
}

impl User {
    /// Removes expired entries from `pending_unbounds`, moving released funds to `released`.
    /// Funds from `released` are ready to be claimed.
    pub fn release_pending(&mut self, info: BlockInfo) {
        // The fact that `pending unbounds are always added to the end, so they are always ordered
        // is assumed here.

        // Nothing waits for unbound
        if self.pending_unbounds.is_empty() {
            return;
        };

        // First item is still not ready for release
        if !self.pending_unbounds[0].release_at.is_expired(&info) {
            return;
        }

        let non_expired_idx = self
            .pending_unbounds
            .partition_point(|pending| pending.release_at.is_expired(&info));

        for pending in &self.pending_unbounds[..non_expired_idx] {
            self.released += pending.amount;
        }

        self.pending_unbounds = self.pending_unbounds[non_expired_idx..].into();
    }
}
