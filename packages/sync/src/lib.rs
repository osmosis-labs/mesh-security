mod locks;
mod range;

pub use locks::{LockError, LockState, Lockable};
pub use range::{max_val, min_val, spread, RangeError, ValueRange};
