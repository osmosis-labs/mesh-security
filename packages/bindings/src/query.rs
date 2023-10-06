use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Coin, CustomQuery, QuerierWrapper, QueryRequest, StdResult};

#[cw_serde]
#[derive(QueryResponses)]
#[query_responses(nested)]
pub enum VirtualStakeCustomQuery {
    VirtualStake(VirtualStakeQuery),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum VirtualStakeQuery {
    /// Returns the available and currently used virtual staking
    /// amounts for the given contract.
    /// If the contract has never been authorized for virtual staking,
    /// it will return zero values rather than an error.
    #[returns(BondStatusResponse)]
    BondStatus { contract: String },

    /// Returns the blockchain's slashing ratios.
    #[returns(SlashRatioResponse)]
    SlashRatio {},
}

/// Bookkeeping info in the virtual staking sdk module
#[cw_serde]
pub struct BondStatusResponse {
    /// Maximum number of tokens than can be minted by this address.
    /// denom is always the native staking token.
    pub cap: Coin,
    /// Number of tokens than already have been minted by this address.
    /// Trying to mint more than (cap - currently_minted) will fail.
    pub delegated: Coin,
}

#[cw_serde]
pub struct SlashRatioResponse {
    /// Slash ratio due to downtime. Used for temporary jailing.
    pub slash_fraction_downtime: String,
    /// Slash ratio due to double signing. Applied when a validator is permanently jailed (tombstoned).
    pub slash_fraction_double_sign: String,
}

impl CustomQuery for VirtualStakeCustomQuery {}

impl From<VirtualStakeQuery> for QueryRequest<VirtualStakeCustomQuery> {
    fn from(query: VirtualStakeQuery) -> Self {
        QueryRequest::Custom(VirtualStakeCustomQuery::VirtualStake(query))
    }
}

/// This is a helper wrapper to easily use our custom queries
pub struct TokenQuerier<'a> {
    querier: &'a QuerierWrapper<'a, VirtualStakeCustomQuery>,
}

impl<'a> TokenQuerier<'a> {
    pub fn new(querier: &'a QuerierWrapper<VirtualStakeCustomQuery>) -> Self {
        TokenQuerier { querier }
    }

    pub fn bond_status(&self, contract: String) -> StdResult<BondStatusResponse> {
        let bond_status_query = VirtualStakeQuery::BondStatus { contract };
        self.querier.query(&bond_status_query.into())
    }

    pub fn slash_ratio(&self) -> StdResult<SlashRatioResponse> {
        let slash_ratio_query = VirtualStakeQuery::SlashRatio {};
        self.querier.query(&slash_ratio_query.into())
    }
}
