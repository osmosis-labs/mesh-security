use cosmwasm_schema::cw_serde;
use thiserror::Error;

#[cw_serde]
pub struct Lockable<T> {
    inner: T,
    lock: LockState,
}

#[cw_serde]
pub enum LockState {
    #[serde(rename = "no")]
    Unlocked,
    #[serde(rename = "w")]
    WriteLocked,
    #[serde(rename = "r")]
    ReadLocked(u32),
}

#[derive(Debug, Error)]
pub enum LockError {
    #[error("Value is already write locked")]
    WriteLocked,
    #[error("Value is already read locked")]
    ReadLocked,
    #[error("Attempt to release a lock which was not held")]
    NoLockHeld,
}

impl Default for LockState {
    fn default() -> Self {
        LockState::Unlocked
    }
}

impl<T> Lockable<T> {
    pub fn new(inner: T) -> Self {
        Lockable {
            inner,
            lock: LockState::Unlocked,
        }
    }

    pub fn read(&self) -> Result<&T, LockError> {
        match self.lock {
            LockState::WriteLocked => Err(LockError::WriteLocked),
            _ => Ok(&self.inner),
        }
    }

    pub fn write(&mut self) -> Result<&mut T, LockError> {
        match self.lock {
            LockState::WriteLocked => Err(LockError::WriteLocked),
            LockState::ReadLocked(_) => Err(LockError::ReadLocked),
            LockState::Unlocked => Ok(&mut self.inner),
        }
    }

    pub fn lock_write(&mut self) -> Result<(), LockError> {
        match self.lock {
            LockState::Unlocked => {
                self.lock = LockState::WriteLocked;
                Ok(())
            }
            LockState::WriteLocked => Err(LockError::WriteLocked),
            LockState::ReadLocked(_) => Err(LockError::ReadLocked),
        }
    }

    pub fn lock_read(&mut self) -> Result<(), LockError> {
        match self.lock {
            LockState::Unlocked => {
                self.lock = LockState::ReadLocked(1);
                Ok(())
            }
            LockState::ReadLocked(x) => {
                self.lock = LockState::ReadLocked(x + 1);
                Ok(())
            }
            LockState::WriteLocked => Err(LockError::WriteLocked),
        }
    }

    pub fn unlock_read(&mut self) -> Result<(), LockError> {
        match self.lock {
            LockState::Unlocked => Err(LockError::NoLockHeld),
            LockState::ReadLocked(1) => {
                self.lock = LockState::Unlocked;
                Ok(())
            }
            LockState::ReadLocked(x) => {
                self.lock = LockState::ReadLocked(x - 1);
                Ok(())
            }
            LockState::WriteLocked => Err(LockError::WriteLocked),
        }
    }

    pub fn unlock_write(&mut self) -> Result<(), LockError> {
        match self.lock {
            LockState::Unlocked => Err(LockError::NoLockHeld),
            LockState::ReadLocked(_) => Err(LockError::ReadLocked),
            LockState::WriteLocked => {
                self.lock = LockState::Unlocked;
                Ok(())
            }
        }
    }
}
