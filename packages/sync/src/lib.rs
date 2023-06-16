mod locks;
mod range;

pub use locks::{LockError, LockState, Lockable};
pub use range::{max_range, min_range, spread, RangeError, ValueRange};
