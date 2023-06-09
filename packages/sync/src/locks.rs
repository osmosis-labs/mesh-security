use cosmwasm_schema::cw_serde;
use thiserror::Error;

#[cw_serde]
pub struct Lockable<T> {
    inner: T,
    lock: LockState,
}

#[cw_serde]
#[derive(Copy)]
pub enum LockState {
    #[serde(rename = "no")]
    Unlocked,
    #[serde(rename = "w")]
    WriteLocked,
    #[serde(rename = "r")]
    ReadLocked(u32),
}

#[derive(Debug, Error, PartialEq, Eq)]
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

    pub fn state(&self) -> LockState {
        self.lock
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_lock_works() {
        // let's try locking and
        let mut lockable = Lockable::new(5u32);
        assert_eq!(lockable.state(), LockState::Unlocked);

        // lock it once
        lockable.lock_write().unwrap();
        assert_eq!(lockable.state(), LockState::WriteLocked);

        // can't lock it again
        let err = lockable.lock_write().unwrap_err();
        assert_eq!(err, LockError::WriteLocked);

        // can't read lock it
        let err = lockable.lock_read().unwrap_err();
        assert_eq!(err, LockError::WriteLocked);

        // unlock it once
        lockable.unlock_write().unwrap();
        assert_eq!(lockable.state(), LockState::Unlocked);

        // can't lock it again
        let err = lockable.unlock_write().unwrap_err();
        assert_eq!(err, LockError::NoLockHeld);
    }

    #[test]
    fn read_lock_works() {
        // let's try locking and
        let mut lockable = Lockable::new(5u32);
        assert_eq!(lockable.state(), LockState::Unlocked);

        // lock it once
        lockable.lock_read().unwrap();
        assert_eq!(lockable.state(), LockState::ReadLocked(1));

        // can't write lock it
        let err = lockable.lock_write().unwrap_err();
        assert_eq!(err, LockError::ReadLocked);

        // can lock it again
        lockable.lock_read().unwrap();
        assert_eq!(lockable.state(), LockState::ReadLocked(2));

        // unlock it once
        lockable.unlock_read().unwrap();
        assert_eq!(lockable.state(), LockState::ReadLocked(1));

        // unlock it twice
        lockable.unlock_read().unwrap();
        assert_eq!(lockable.state(), LockState::Unlocked);

        // can't unlock more than there were locks
        let err = lockable.unlock_read().unwrap_err();
        assert_eq!(err, LockError::NoLockHeld);
    }

    #[test]
    fn write_lock_enforces_access() {
        let mut lockable = Lockable::new(5u32);
        lockable.lock_write().unwrap();

        // cannot read nor write this data
        let err = lockable.read().unwrap_err();
        assert_eq!(err, LockError::WriteLocked);

        let err = lockable.write().unwrap_err();
        assert_eq!(err, LockError::WriteLocked);
    }

    #[test]
    fn read_lock_enforces_access() {
        let mut lockable = Lockable::new(5u32);
        lockable.lock_read().unwrap();

        // can read this data
        let val = lockable.read().unwrap();
        assert_eq!(*val, 5u32);

        // cannot write this data
        let err = lockable.write().unwrap_err();
        assert_eq!(err, LockError::ReadLocked);
    }

    #[test]
    fn modify_unlocked_number() {
        let mut lockable = Lockable::new(5u32);

        // update the data via deref
        *lockable.write().unwrap() = 6u32;
        assert_eq!(*lockable.read().unwrap(), 6u32);

        // update the data via method
        *lockable.write().unwrap() += 10;
        assert_eq!(*lockable.read().unwrap(), 16u32);
    }

    #[derive(Debug, PartialEq, Eq)]
    struct TestStruct {
        a: u32,
        b: u32,
    }

    impl TestStruct {
        fn new(a: u32, b: u32) -> Self {
            Self { a, b }
        }

        fn multiply_vals(&mut self) {
            self.b *= self.a;
        }
    }

    #[test]
    fn modify_unlocked_struct() {
        let mut lockable = Lockable::new(TestStruct::new(5, 10));

        // update a field
        lockable.write().unwrap().a = 8u32;
        assert_eq!(lockable.read().unwrap(), &TestStruct::new(8, 10));

        // call a method
        lockable.write().unwrap().multiply_vals();
        assert_eq!(lockable.read().unwrap(), &TestStruct::new(8, 80));
    }
}
