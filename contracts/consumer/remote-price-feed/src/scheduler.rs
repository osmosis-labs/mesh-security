use cosmwasm_std::{DepsMut, Env, Response, Timestamp};
use cw_storage_plus::Item;

use crate::error::ContractError;

pub trait Action: Fn(DepsMut, &Env) -> Result<Response, ContractError> {}
impl<F> Action for F where F: Fn(DepsMut, &Env) -> Result<Response, ContractError> {}

/// A helper to schedule a single action to be executed regularly,
/// as in "every epoch". It relies on a trigger being called rather rapidly (every block?).
pub struct Scheduler<A> {
    last_epoch: Item<'static, Timestamp>,
    epoch_in_secs: Item<'static, u64>,
    action: A,
}

impl<A> Scheduler<A>
where
    A: Action,
{
    pub const fn new(action: A) -> Self {
        Self {
            last_epoch: Item::new("last_epoch"),
            epoch_in_secs: Item::new("epoch"),
            action,
        }
    }

    pub fn init(&self, deps: &mut DepsMut, epoch_in_secs: u64) -> Result<(), ContractError> {
        self.last_epoch
            .save(deps.storage, &Timestamp::from_seconds(0))?;
        self.epoch_in_secs.save(deps.storage, &epoch_in_secs)?;
        Ok(())
    }

    pub fn trigger(&self, deps: DepsMut, env: &Env) -> Result<Response, ContractError> {
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
    use super::*;
    use cosmwasm_std::{
        testing::{mock_dependencies, mock_env},
        Binary,
    };

    #[test]
    fn scheduler_first_epoch_always_fires() {
        let scheduler = Scheduler::new(|_, _| Ok(Response::new().set_data(Binary::from(b"foo"))));
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
        let scheduler = Scheduler::new(|_, _| Ok(Response::new().set_data(Binary::from(b"foo"))));
        let mut deps = mock_dependencies();
        let mut env = mock_env();

        scheduler.init(&mut deps.as_mut(), 10).unwrap();

        #[track_caller]
        fn assert_fired<A: Action>(s: &Scheduler<A>, deps: DepsMut, env: &Env) {
            assert!(s.trigger(deps, env).unwrap().data.is_some())
        }

        #[track_caller]
        fn assert_noop<A: Action>(s: &Scheduler<A>, deps: DepsMut, env: &Env) {
            assert!(s.trigger(deps, env).unwrap().data.is_none())
        }

        assert_fired(&scheduler, deps.as_mut(), &env);

        env.block.time = env.block.time.plus_seconds(5);
        assert_noop(&scheduler, deps.as_mut(), &env);
        env.block.time = env.block.time.plus_seconds(5);
        assert_fired(&scheduler, deps.as_mut(), &env);
        env.block.time = env.block.time.plus_seconds(5);
        assert_noop(&scheduler, deps.as_mut(), &env);
        env.block.time = env.block.time.plus_seconds(5);
        assert_fired(&scheduler, deps.as_mut(), &env);
    }
}
