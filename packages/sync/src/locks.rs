use cosmwasm_schema::cw_serde;
use thiserror::Error;

#[cw_serde]
#[derive(Default)]
pub struct Lockable<T> {
    inner: T,
    lock: LockState,
}

#[cw_serde]
#[derive(Copy, Default)]
pub enum LockState {
    #[default]
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

#[cfg(test)]
mod tests_plus {
    use super::*;

    use cosmwasm_std::{testing::MockStorage, StdError, Uint128};
    use cw_storage_plus::{Item, Map};

    #[cw_serde]
    pub struct Person {
        name: String,
        age: u32,
    }

    impl Person {
        pub fn new(name: &str, age: u32) -> Self {
            Self {
                name: name.to_string(),
                age,
            }
        }
    }

    #[derive(Error, Debug, PartialEq)]
    pub enum TestsError {
        #[error("{0}")]
        Std(#[from] StdError),
        #[error("{0}")]
        Lock(#[from] LockError),
    }

    #[cw_serde]
    #[derive(Default)]
    pub struct UserInfo {
        // User collateral
        pub collateral: Uint128,
        // Highest user lien
        pub max_lien: Uint128,
        // Total slashable amount for user
        pub total_slashable: Uint128,
    }

    #[test]
    fn modify_item_with_locks() {
        let mut store = MockStorage::new();
        const PERSON: Item<Lockable<Person>> = Item::new("person");

        // store a normal unlocked person
        PERSON
            .save(&mut store, &Lockable::new(Person::new("John", 32)))
            .unwrap();

        // had a birthday
        PERSON
            .update(&mut store, |mut p| {
                p.write()?.age += 1;
                Ok::<_, TestsError>(p)
            })
            .unwrap();

        // get this and read-lock it
        PERSON
            .update(&mut store, |mut p| {
                assert_eq!(p.read()?.age, 33);
                p.lock_read()?;
                Ok::<_, TestsError>(p)
            })
            .unwrap();

        // error trying to write it later
        let mut p = PERSON.load(&store).unwrap();
        let err = p.write().unwrap_err();
        assert_eq!(err, LockError::ReadLocked);

        // but we can unlock and save it to make to release
        p.unlock_read().unwrap();
        PERSON.save(&mut store, &p).unwrap();

        // now, it is fine to change again
        PERSON
            .update(&mut store, |mut p| {
                p.write()?.age += 1;
                assert_eq!(p.read()?.age, 34);
                Ok::<_, TestsError>(p)
            })
            .unwrap();
    }

    #[test]
    fn map_methods_with_locks() {
        let mut store = MockStorage::new();
        const AGES: Map<&str, Lockable<Uint128>> = Map::new("people");

        // add a few people
        AGES.save(&mut store, "John", &Lockable::new(Uint128::new(32)))
            .unwrap();
        AGES.save(&mut store, "Maria", &Lockable::new(Uint128::new(47)))
            .unwrap();

        // We can edit unlocked person
        AGES.update(&mut store, "John", |p| {
            let mut p = p.unwrap_or_default();
            *p.write()? += Uint128::new(1);
            Ok::<_, TestsError>(p)
        })
        .unwrap();

        // Update works on new values, setting to unlocked by default
        AGES.update(&mut store, "Wilber", |p| {
            let mut p = p.unwrap_or_default();
            *p.write()? += Uint128::new(2);
            Ok::<_, TestsError>(p)
        })
        .unwrap();

        // We can range over them well
        let total_age = AGES
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .fold(Ok(Uint128::zero()), |sum, item| {
                Ok::<_, TestsError>(sum? + *item?.1.read()?)
            })
            .unwrap();
        assert_eq!(total_age, Uint128::new(33 + 47 + 2));

        // We can get count
        let num_people = AGES
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .count();
        assert_eq!(num_people, 3);

        // Read-lock John
        let mut j = AGES.load(&store, "John").unwrap();
        j.lock_read().unwrap();
        AGES.save(&mut store, "John", &j).unwrap();

        // We can no longer edit it
        let err = AGES
            .update(&mut store, "John", |p| {
                let mut p = p.unwrap_or_default();
                *p.write()? += Uint128::new(1);
                Ok::<_, TestsError>(p)
            })
            .unwrap_err();
        assert_eq!(err, TestsError::Lock(LockError::ReadLocked));

        // We can still range over all
        let total_age = AGES
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .fold(Ok(Uint128::zero()), |sum, item| {
                Ok::<_, TestsError>(sum? + *item?.1.read()?)
            })
            .unwrap();
        assert_eq!(total_age, Uint128::new(33 + 47 + 2));

        // We can get count
        let num_people = AGES
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .count();
        assert_eq!(num_people, 3);

        // Write-lock Wilber
        let mut w = AGES.load(&store, "Wilber").unwrap();
        w.lock_write().unwrap();
        AGES.save(&mut store, "Wilber", &w).unwrap();

        // We cannot range over all
        let err = AGES
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .fold(Ok(Uint128::zero()), |sum, item| {
                Ok::<_, TestsError>(sum? + *item?.1.read()?)
            })
            .unwrap_err();
        assert_eq!(err, TestsError::Lock(LockError::WriteLocked));

        // We can get count (kind of edge case bug, but I don't think we can change this or it matters)
        let num_people = AGES
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .count();
        assert_eq!(num_people, 3);
    }

    #[test]
    fn map_methods_with_locked_struct() {
        let mut store = MockStorage::new();
        const USERS: Map<&str, Lockable<UserInfo>> = Map::new("users");

        // add a few people
        USERS
            .save(
                &mut store,
                "John",
                &Lockable::new(UserInfo {
                    collateral: Default::default(),
                    max_lien: Default::default(),
                    total_slashable: Default::default(),
                }),
            )
            .unwrap();
        USERS
            .save(
                &mut store,
                "Maria",
                &Lockable::new(UserInfo {
                    collateral: Uint128::new(1),
                    max_lien: Uint128::new(2),
                    total_slashable: Uint128::new(3),
                }),
            )
            .unwrap();

        // Update works on new values, setting to unlocked by default
        USERS
            .update(&mut store, "Wilber", |p| {
                let mut p = p.unwrap_or_default();
                *p.write()? = UserInfo {
                    collateral: Uint128::new(4),
                    max_lien: Uint128::new(5),
                    total_slashable: Uint128::new(6),
                };
                Ok::<_, TestsError>(p)
            })
            .unwrap();

        // Read-lock John
        let mut j = USERS.load(&store, "John").unwrap();
        j.lock_read().unwrap();
        USERS.save(&mut store, "John", &j).unwrap();

        // We can still range over all
        let total_collateral = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .fold(Ok(Uint128::zero()), |sum, item| {
                Ok::<_, TestsError>(sum? + item?.1.read()?.collateral)
            })
            .unwrap();
        assert_eq!(total_collateral, Uint128::new(1 + 4));

        // We can get count
        let num_users = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .count();
        assert_eq!(num_users, 3);

        // Write-lock Wilber
        let mut w = USERS.load(&store, "Wilber").unwrap();
        w.lock_write().unwrap();
        USERS.save(&mut store, "Wilber", &w).unwrap();

        // We cannot range over all
        let err = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .fold(Ok(Uint128::zero()), |sum, item| {
                Ok::<_, TestsError>(sum? + item?.1.read()?.max_lien)
            })
            .unwrap_err();
        assert_eq!(err, TestsError::Lock(LockError::WriteLocked));

        // We cannot range and map over all values either
        let err = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .map(|item| {
                let (_, user_lock) = item?;
                let user = user_lock.read()?;
                Ok(user.collateral)
            })
            .collect::<Result<Vec<_>, TestsError>>()
            .unwrap_err();
        assert_eq!(err, TestsError::Lock(LockError::WriteLocked));

        // But we can re-map (perhaps not a good idea) the write-locked values
        let collaterals = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .map(|item| {
                let (_, user_lock) = item?;
                let res = user_lock.read();
                match res {
                    Ok(user) => Ok(user.collateral),
                    Err(LockError::WriteLocked) => Ok(Uint128::zero()),
                    Err(e) => Err(e.into()),
                }
            })
            .collect::<Result<Vec<_>, TestsError>>()
            .unwrap();
        assert_eq!(collaterals.len(), 3);

        // Or we can skip (perhaps not a good idea either) the write-locked values
        let collaterals = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .filter(|item| match item {
                Ok((_, user_lock)) => {
                    let res = user_lock.read();
                    match res {
                        Ok(_) => true,
                        Err(LockError::WriteLocked) => false,
                        Err(_) => true,
                    }
                }
                Err(_) => true,
            })
            .map(|item| {
                let (_, user_lock) = item?;
                let user = user_lock.read()?;
                Ok(user.collateral)
            })
            .collect::<Result<Vec<_>, TestsError>>()
            .unwrap();
        assert_eq!(collaterals.len(), 2);

        // We can get count (kind of edge case bug, but I don't think we can change this or it matters)
        let num_users = USERS
            .range(&store, None, None, cosmwasm_std::Order::Ascending)
            .count();
        assert_eq!(num_users, 3);
    }
}
