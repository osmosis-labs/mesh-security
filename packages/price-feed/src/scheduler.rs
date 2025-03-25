use std::marker::PhantomData;

use cosmwasm_std::{DepsMut, Env, Response, StdError, Timestamp};
use cw_storage_plus::Item;

pub trait Action<Error>: Fn(DepsMut, &Env) -> Result<Response, Error> {}
impl<F, Error> Action<Error> for F where F: Fn(DepsMut, &Env) -> Result<Response, Error> {}

/// A component that schedules a single action to be executed regularly,
/// as in "every epoch". It relies on a trigger being called rather rapidly (every block?).
pub struct Scheduler<A, Error> {
    last_epoch: Item<Timestamp>,
    epoch_in_secs: Item<u64>,
    action: A,
    _phantom_data: PhantomData<Error>, // Add a PhantomData to mark the error type
}

impl<A, E> Scheduler<A, E>
where
    A: Action<E>,
    E: From<StdError>,
{
    pub const fn new(action: A) -> Self {
        Self {
            last_epoch: Item::new("last_epoch"),
            epoch_in_secs: Item::new("epoch"),
            action,
            _phantom_data: PhantomData, // initialize PhantomData
        }
    }

    pub fn init(&self, deps: &mut DepsMut, epoch_in_secs: u64) -> Result<(), E> {
        self.last_epoch
            .save(deps.storage, &Timestamp::from_seconds(0))?;
        self.epoch_in_secs.save(deps.storage, &epoch_in_secs)?;
        Ok(())
    }

    pub fn trigger(&self, deps: DepsMut, env: &Env) -> Result<Response, E> {
        let last_epoch = self.last_epoch.load(deps.storage)?;
        let epoch_in_secs = self.epoch_in_secs.load(deps.storage)?;
        let secs_since_last_epoch = env.block.time.seconds() - last_epoch.seconds();
        if secs_since_last_epoch >= epoch_in_secs {
            self.last_epoch.save(deps.storage, &env.block.time)?;
            (self.action)(deps, env)
        } else {
            Ok(Response::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::StdError;

    use super::*;
    use cosmwasm_std::{
        testing::{mock_dependencies, mock_env},
        Binary,
    };

    type TestScheduler = Scheduler<Box<dyn Action<StdError>>, StdError>;

    #[test]
    fn scheduler_first_epoch_always_fires() {
        let scheduler = TestScheduler::new(Box::new(|_, _| {
            Ok(Response::new().set_data(Binary::from(b"foo")))
        }));
        let mut deps = mock_dependencies();
        let env = mock_env();

        scheduler.init(&mut deps.as_mut(), 111111).unwrap();
        assert!(scheduler
            .trigger(deps.as_mut(), &env)
            .unwrap()
            .data
            .is_some());
    }

    #[test]
    fn scheduler() {
        let scheduler = TestScheduler::new(Box::new(|_, _| {
            Ok(Response::new().set_data(Binary::from(b"foo")))
        }));
        let mut deps = mock_dependencies();
        let mut env = mock_env();

        scheduler.init(&mut deps.as_mut(), 10).unwrap();

        #[track_caller]
        fn assert_fired<A, E>(s: &Scheduler<A, E>, deps: DepsMut, env: &Env)
        where
            A: Action<E>,
            E: std::fmt::Debug + From<StdError>,
        {
            assert!(s.trigger(deps, env).unwrap().data.is_some())
        }

        #[track_caller]
        fn assert_noop<A, E>(s: &Scheduler<A, E>, deps: DepsMut, env: &Env)
        where
            A: Action<E>,
            E: std::fmt::Debug + From<StdError>,
        {
            assert_eq!(s.trigger(deps, env).unwrap(), Response::new())
        }

        assert_fired(&scheduler, deps.as_mut(), &env);

        env.block.time = env.block.time.plus_seconds(5);
        assert_noop(&scheduler, deps.as_mut(), &env);
        env.block.time = env.block.time.plus_seconds(5);
        assert_fired(&scheduler, deps.as_mut(), &env);
        env.block.time = env.block.time.plus_seconds(5);
        assert_noop(&scheduler, deps.as_mut(), &env);
        assert_noop(&scheduler, deps.as_mut(), &env);
        env.block.time = env.block.time.plus_seconds(5);
        assert_fired(&scheduler, deps.as_mut(), &env);
    }
}
