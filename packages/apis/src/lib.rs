mod cross_staking;
mod local_staking;
mod vault;

pub use cross_staking::CrossStakingApi;
pub use local_staking::{LocalStakingApi, MaxSlashResponse, QueryMsg as LocalStakingQueryMsg};
pub use vault::VaultApi;
