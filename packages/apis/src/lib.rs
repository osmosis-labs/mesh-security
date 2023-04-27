mod local_staking;
mod remote_staking;
mod vault;

pub use local_staking::{LocalStakingApi, MaxSlashResponse};
pub use remote_staking::RemoteStakingApi;
pub use vault::VaultApi;
