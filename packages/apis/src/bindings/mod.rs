mod msg;
mod query;

pub use msg::{VirtualStakeCustomMsg, VirtualStakeMsg};
pub use query::{BondStatusResponse, TokenQuerier, VirtualStakeCustomQuery, VirtualStakeQuery};

// This is a signal, such that any contract that imports these helpers
// will only run on blockchains that support virtual_staking feature
#[no_mangle]
extern "C" fn requires_virtual_staking() {}
