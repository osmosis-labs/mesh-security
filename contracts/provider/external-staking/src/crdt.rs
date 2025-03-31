use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Order, StdError, StdResult, Storage};
use cw_storage_plus::{Bound, Map};
use std::cmp::max;

#[cw_serde]
/// ValidatorState maintains a sorted list of updates with no duplicates.
/// The first one is the one with the highest start_height.
pub struct ValidatorState(Vec<ValState>);

impl ValidatorState {
    /// Add one more element to this list, maintaining the constraints
    pub fn insert_unique(&mut self, update: ValState) {
        self.0.push(update);
        self.0.sort_by(|a, b| b.start_height.cmp(&a.start_height));
        self.0.dedup();
    }

    pub fn query_at_height(&self, height: u64) -> Option<&ValState> {
        self.0.iter().find(|u| u.start_height <= height)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get_state(&self) -> State {
        if self.is_empty() {
            State::Unknown {}
        } else {
            self.0[0].state
        }
    }

    pub fn is_active(&self) -> bool {
        !self.is_empty() && self.0[0].state == State::Active {}
    }

    pub fn is_tombstoned(&self) -> bool {
        !self.is_empty() && self.0[0].state == State::Tombstoned {}
    }

    fn drain_newer(&mut self, height: u64) {
        if self.0.is_empty() || self.0[0].start_height < height {
            return;
        }
        let invalidated_idx = self.0.partition_point(|v| v.start_height >= height);
        self.0.drain(..invalidated_idx);
    }

    fn drain_older(&mut self, time: u64) {
        if self.0.is_empty() {
            return;
        }
        let old_idx = max(1, self.0.partition_point(|v| v.start_time > time));
        self.0.drain(old_idx..);
    }
}

#[cw_serde]
pub struct ValState {
    pub pub_key: String,
    pub start_height: u64,
    pub start_time: u64,
    pub state: State,
}

#[cw_serde]
#[derive(Copy)]
pub enum State {
    /// Validator is part of the validator set.
    Active {},
    /// Validator is not part of the validator set due to being unbonded.
    Unbonded {},
    /// Validator is not part of the validator set due to being jailed.
    Jailed {},
    /// Validator is not part of the validator set due to being tombstoned.
    Tombstoned {},
    /// Validator is in the map but we don't have info about its state.
    Unknown {},
}

impl ValState {
    pub fn new(pub_key: impl Into<String>, start_height: u64, start_time: u64) -> Self {
        ValState {
            pub_key: pub_key.into(),
            start_height,
            start_time,
            state: State::Active {},
        }
    }
}

/// This holds all CRDT related state and logic (related to validators)
pub struct CrdtState {
    validators: Map<String, ValidatorState>,
}

impl CrdtState {
    pub const fn new() -> Self {
        CrdtState {
            validators: Map::new("crdt.validators"),
        }
    }

    /// Add a validator and set it to active.
    /// If the validator is tombstoned, this does nothing.
    /// If the validator already exists, it will be set to `Active`, and its pubkey updated.
    /// In test code, this is called from `test_set_active_validator`.
    /// In non-test code, this is called from `ibc_packet_receive`
    pub fn add_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        pub_key: &str,
        height: u64,
        time: u64,
    ) -> Result<(), StdError> {
        let mut validator_state = self
            .validators
            .may_load(storage, valoper.to_string())?
            .unwrap_or_else(|| ValidatorState(vec![]));

        if !validator_state.is_tombstoned() {
            let val_state = ValState {
                pub_key: pub_key.to_string(),
                start_height: height,
                start_time: time,
                state: State::Active {},
            };
            validator_state.insert_unique(val_state);
            self.validators
                .save(storage, valoper.to_string(), &validator_state)?;
        }
        Ok(())
    }

    /// Update a validator.
    /// If the validator does not exist, or it is tombstoned, it does nothing.
    /// In non-test code, this is called from `ibc_packet_receive`
    pub fn update_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        pub_key: &str,
        height: u64,
        time: u64,
    ) -> Result<(), StdError> {
        let mut validator_state = self
            .validators
            .may_load(storage, valoper.to_string())?
            .unwrap_or_else(|| ValidatorState(vec![]));

        // We just silently ignore it if does not exist, or tombstoned
        if !validator_state.is_empty() && !validator_state.is_tombstoned() {
            // Copy state from previous entry. This is not commutative anymore :-/
            let old = self.validator_at_height(storage, valoper, height)?;
            // Ignore if not previous state at height (should not happen)
            if old.is_none() {
                return Ok(());
            }
            let old = old.unwrap();
            let val_state = ValState {
                pub_key: pub_key.to_string(),
                start_height: height,
                start_time: time,
                state: old.state,
            };
            validator_state.insert_unique(val_state);
            self.validators
                .save(storage, valoper.to_string(), &validator_state)?;
        }
        Ok(())
    }

    /// Remove a validator from the active set.
    /// If the validator does not exist, or it is tombstoned, it does nothing.
    /// In non-test code, this is called from `ibc_packet_receive`
    pub fn remove_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        height: u64,
        time: u64,
    ) -> Result<(), StdError> {
        let mut validator_state = self
            .validators
            .may_load(storage, valoper.to_string())?
            .unwrap_or_else(|| ValidatorState(vec![]));

        // We just silently ignore it if does not exist, or tombstoned
        if !validator_state.is_empty() && !validator_state.is_tombstoned() {
            // Copy data from previous entry. This is not commutative anymore :-/
            let old = self.validator_at_height(storage, valoper, height)?;
            // Ignore if not previous entry at height (should not happen)
            if old.is_none() {
                return Ok(());
            }
            let old = old.unwrap();
            let val_state = ValState {
                pub_key: old.pub_key,
                start_height: height,
                start_time: time,
                state: State::Unbonded {},
            };
            validator_state.insert_unique(val_state);
            self.validators
                .save(storage, valoper.to_string(), &validator_state)?;
        }
        Ok(())
    }

    /// Remove a validator from the active set due to jailing.
    /// If the validator does not exist, or it is tombstoned, it does nothing.
    /// In non-test code, this is called from `ibc_packet_receive`
    pub fn jail_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        height: u64,
        time: u64,
    ) -> Result<(), StdError> {
        let mut validator_state = self
            .validators
            .may_load(storage, valoper.to_string())?
            .unwrap_or_else(|| ValidatorState(vec![]));

        // We just silently ignore it if does not exist, or tombstoned
        if !validator_state.is_empty() && !validator_state.is_tombstoned() {
            // Copy data from previous entry. This is not commutative anymore :-/
            let old = self.validator_at_height(storage, valoper, height)?;
            // Ignore if not previous entry at height (should not happen)
            if old.is_none() {
                return Ok(());
            }
            let old = old.unwrap();
            let val_state = ValState {
                pub_key: old.pub_key,
                start_height: height,
                start_time: time,
                state: State::Jailed {},
            };
            validator_state.insert_unique(val_state);
            self.validators
                .save(storage, valoper.to_string(), &validator_state)?;
        }
        Ok(())
    }

    /// Tombstone a validator.
    /// In non-test code, this is called from `ibc_packet_receive`
    pub fn tombstone_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        height: u64,
        time: u64,
    ) -> Result<(), StdError> {
        let mut validator_state = self
            .validators
            .may_load(storage, valoper.to_string())?
            .unwrap_or_else(|| ValidatorState(vec![]));

        // We just silently ignore it if already tombstoned
        if !validator_state.is_tombstoned() {
            // Drain events that are newer than `height` (this is the final registered event)
            validator_state.drain_newer(height);

            // Insert tombstoning
            let val_state = ValState {
                pub_key: "".to_string(), // FIXME? Keep pubkey
                start_height: height,
                start_time: time,
                state: State::Tombstoned {},
            };
            validator_state.insert_unique(val_state);

            self.validators
                .save(storage, valoper.to_string(), &validator_state)?;
        }
        Ok(())
    }

    pub fn is_active_validator(&self, storage: &dyn Storage, valoper: &str) -> StdResult<bool> {
        let active = self
            .validators
            .may_load(storage, valoper.to_string())?
            .map(|s| s.is_active())
            .unwrap_or(false);
        Ok(active)
    }

    pub fn is_active_validator_at_height(
        &self,
        storage: &dyn Storage,
        valoper: &str,
        height: u64,
    ) -> StdResult<bool> {
        let active = self
            .active_validator_at_height(storage, valoper, height)?
            .is_some();
        Ok(active)
    }

    /// This returns the valoper address of all active validators
    pub fn list_active_validators(
        &self,
        storage: &dyn Storage,
        start_after: Option<&str>,
        limit: usize,
    ) -> StdResult<Vec<String>> {
        let start = start_after.map(Bound::exclusive);
        self.validators
            .range(storage, start, None, Order::Ascending)
            .filter_map(|r| match r {
                Ok((valoper, validator_state)) if validator_state.is_active() => Some(Ok(valoper)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .take(limit)
            .collect()
    }

    /// This returns the valoper address and latest state of all validators we are aware of
    pub fn list_validators(
        &self,
        storage: &dyn Storage,
        start_after: Option<&str>,
        limit: usize,
    ) -> StdResult<Vec<(String, State)>> {
        let start = start_after.map(Bound::exclusive);
        self.validators
            .range(storage, start, None, Order::Ascending)
            .map(|r| match r {
                Ok((valoper, state)) => Ok((valoper, state.get_state())),
                Err(e) => Err(e),
            })
            .take(limit)
            .collect()
    }

    pub fn active_validator(
        &self,
        storage: &dyn Storage,
        valoper: &str,
    ) -> StdResult<Option<ValState>> {
        let state = self.validators.may_load(storage, valoper.to_string())?;
        match state {
            Some(state) if state.is_active() => Ok(state.0.first().cloned()),
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    pub fn active_validator_at_height(
        &self,
        storage: &dyn Storage,
        valoper: &str,
        height: u64,
    ) -> StdResult<Option<ValState>> {
        let state = self.validator_at_height(storage, valoper, height)?;
        match state {
            Some(val_state) if val_state.state == State::Active {} => Ok(Some(val_state)),
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    pub fn validator_at_height(
        &self,
        storage: &dyn Storage,
        valoper: &str,
        height: u64,
    ) -> StdResult<Option<ValState>> {
        let state = self.validators.may_load(storage, valoper.to_string())?;
        match state {
            Some(state) => match state.query_at_height(height) {
                Some(val_state) => Ok(Some(val_state.clone())),
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    pub fn drain_older(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        time: u64,
    ) -> StdResult<()> {
        let mut validator_state = self
            .validators
            .may_load(storage, valoper.to_string())?
            .unwrap_or_else(|| ValidatorState(vec![]));
        if validator_state.0.len() <= 1 {
            return Ok(());
        }
        validator_state.drain_older(time);
        self.validators
            .save(storage, valoper.to_string(), &validator_state)?;
        Ok(())
    }

    pub fn validator_state(&self, storage: &dyn Storage, valoper: &str) -> StdResult<State> {
        Ok(self
            .validators
            .may_load(storage, valoper.to_string())?
            .map(|state| state.get_state())
            .unwrap_or(State::Unknown {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use cosmwasm_std::MemoryStorage;

    // We add three new validators, and remove one
    #[test]
    fn happy_path() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "alice_pub_key", 123, 1234)
            .unwrap();
        crdt.add_validator(&mut storage, "bob", "bob_pub_key", 200, 2345)
            .unwrap();
        crdt.add_validator(&mut storage, "carl", "carl_pub_key", 303, 3456)
            .unwrap();
        crdt.tombstone_validator(&mut storage, "bob", 201, 2346)
            .unwrap();

        assert!(crdt.is_active_validator(&storage, "alice").unwrap());
        assert!(!crdt.is_active_validator(&storage, "bob").unwrap());
        assert!(crdt.is_active_validator(&storage, "carl").unwrap());

        let active = crdt.list_active_validators(&storage, None, 10).unwrap();
        assert_eq!(active, vec!["alice".to_string(), "carl".to_string()]);

        let validators = crdt.list_validators(&storage, None, 10).unwrap();
        assert_eq!(
            validators,
            vec![
                ("alice".to_string(), State::Active {}),
                ("bob".to_string(), State::Tombstoned {}),
                ("carl".to_string(), State::Active {}),
            ]
        );
    }

    // Like happy path, but we remove bob before he was ever added
    #[test]
    fn tombstone_before_add_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.tombstone_validator(&mut storage, "bob", 199, 2344)
            .unwrap();
        crdt.add_validator(&mut storage, "alice", "pk_a", 123, 1234)
            .unwrap();
        crdt.add_validator(&mut storage, "bob", "pk_b", 200, 2345)
            .unwrap();
        crdt.add_validator(&mut storage, "carl", "pk_c", 303, 3459)
            .unwrap();

        assert!(crdt.is_active_validator(&storage, "alice").unwrap());
        assert!(!crdt.is_active_validator(&storage, "bob").unwrap());
        assert!(crdt.is_active_validator(&storage, "carl").unwrap());

        let active = crdt.list_active_validators(&storage, None, 10).unwrap();
        assert_eq!(active, vec!["alice".to_string(), "carl".to_string()]);

        let validators = crdt.list_validators(&storage, None, 10).unwrap();
        assert_eq!(
            validators,
            vec![
                ("alice".to_string(), State::Active {}),
                ("bob".to_string(), State::Tombstoned {}),
                ("carl".to_string(), State::Active {}),
            ]
        );
    }

    // add and remove many validators, then iterate over them
    #[test]
    fn pagination_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        // use two digits so numeric and alphabetic sort match (-2 is after -11, but -02 is before -11)
        let mut validators: Vec<_> = (0..20).map(|i| format!("validator-{:02}", i)).collect();
        for v in &validators {
            crdt.add_validator(&mut storage, v, &format!("{}-pubkey", v), 123, 1234)
                .unwrap();
        }
        // in reverse order, so remove doesn't shift the indexes we will later read
        for i in [19, 17, 12, 11, 7, 4, 3] {
            crdt.tombstone_validator(&mut storage, &validators[i], 200, 2345)
                .unwrap();
            validators.remove(i);
        }

        // total of 13 if we get them all
        let active = crdt.list_active_validators(&storage, None, 20).unwrap();
        assert_eq!(active, validators);
        assert_eq!(active.len(), 13);

        // paginate by 7
        let active = crdt.list_active_validators(&storage, None, 7).unwrap();
        assert_eq!(active.len(), 7);
        assert_eq!(active, validators[0..7]);
        let active = crdt
            .list_active_validators(&storage, Some(&active[6]), 7)
            .unwrap();
        assert_eq!(active.len(), 6);
        assert_eq!(active, validators[7..]);
        let active = crdt
            .list_active_validators(&storage, Some(&active[5]), 7)
            .unwrap();
        assert_eq!(active, Vec::<String>::new());
    }

    #[test]
    fn key_rotation_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "alice_pubkey_1", 123, 1234)
            .unwrap();
        crdt.add_validator(&mut storage, "bob", "bob_pubkey_1", 200, 2345)
            .unwrap();
        // Add does update
        crdt.add_validator(&mut storage, "alice", "alice_pubkey_2", 202, 2347)
            .unwrap();
        // Update does as well
        crdt.update_validator(&mut storage, "alice", "alice_pubkey_3", 203, 2348)
            .unwrap();

        // Query before update
        let alice = crdt
            .active_validator_at_height(&storage, "alice", 200)
            .unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(),
                start_height: 123,
                start_time: 1234,
                state: State::Active {}
            })
        );

        // Query at 2nd add height
        let alice = crdt
            .active_validator_at_height(&storage, "alice", 202)
            .unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_2".to_string(),
                start_height: 202,
                start_time: 2347,
                state: State::Active {}
            })
        );

        // Query after last update height
        let alice = crdt
            .active_validator_at_height(&storage, "alice", 500)
            .unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_3".to_string(),
                start_height: 203,
                start_time: 2348,
                state: State::Active {}
            })
        );
    }

    #[test]
    fn add_existing_validator_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "alice_pubkey_1", 123, 1234)
            .unwrap();
        // Remove changes state
        crdt.remove_validator(&mut storage, "alice", 202, 2347)
            .unwrap();

        // Query before remove height
        let alice = crdt.validator_at_height(&storage, "alice", 200).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(),
                start_height: 123,
                start_time: 1234,
                state: State::Active {}
            })
        );

        // Query at remove height
        let alice = crdt.validator_at_height(&storage, "alice", 202).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(),
                start_height: 202,
                start_time: 2347,
                state: State::Unbonded {}
            })
        );

        // Add it again
        crdt.add_validator(&mut storage, "alice", "alice_pubkey_2", 300, 3456)
            .unwrap();

        // Query after last addition height
        let alice = crdt.validator_at_height(&storage, "alice", 500).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_2".to_string(), // Pubkey has been updated
                start_height: 300,
                start_time: 3456,
                state: State::Active {} // Validator is active
            })
        );
    }

    #[test]
    fn jail_validator_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "alice_pubkey_1", 100, 1234)
            .unwrap();
        // Jail changes state
        crdt.jail_validator(&mut storage, "alice", 200, 2345)
            .unwrap();

        // Query at jail height
        let alice = crdt.validator_at_height(&storage, "alice", 200).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(),
                start_height: 200,
                start_time: 2345,
                state: State::Jailed {}
            })
        );

        // Unjail it to active
        crdt.add_validator(&mut storage, "alice", "alice_pubkey_1", 300, 3456)
            .unwrap();

        // Query after unjailing addition height
        let alice = crdt.validator_at_height(&storage, "alice", 500).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(), // Pubkey has been updated
                start_height: 300,
                start_time: 3456,
                state: State::Active {} // Validator is active again
            })
        );
    }

    #[test]
    fn jail_remove_validator_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "alice_pubkey_1", 100, 1234)
            .unwrap();
        // Jail changes state
        crdt.jail_validator(&mut storage, "alice", 200, 2345)
            .unwrap();

        // Query at jail height
        let alice = crdt.validator_at_height(&storage, "alice", 200).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(),
                start_height: 200,
                start_time: 2345,
                state: State::Jailed {}
            })
        );

        // Remove it instead of unjailing it to active
        crdt.remove_validator(&mut storage, "alice", 300, 3456)
            .unwrap();

        // Query after remove addition height
        let alice = crdt.validator_at_height(&storage, "alice", 500).unwrap();
        assert_eq!(
            alice,
            Some(ValState {
                pub_key: "alice_pubkey_1".to_string(),
                start_height: 300,
                start_time: 3456,
                state: State::Unbonded {}
            })
        );
    }

    #[test]
    fn tombstone_before_all_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "pk_a", 100, 1234)
            .unwrap();
        crdt.tombstone_validator(&mut storage, "bob", 100, 1234)
            .unwrap();
        crdt.add_validator(&mut storage, "bob", "pk_b", 200, 2345)
            .unwrap();

        assert!(!crdt.is_active_validator(&storage, "bob").unwrap());

        let active = crdt.list_active_validators(&storage, None, 10).unwrap();
        assert_eq!(active, vec!["alice".to_string()]);

        // Bob is not active, but we can still query him
        let bob = crdt.validator_at_height(&storage, "bob", 500).unwrap();
        assert_eq!(
            bob,
            Some(ValState {
                pub_key: "".to_string(),
                start_height: 100,
                start_time: 1234,
                state: State::Tombstoned {}
            })
        );

        // Querying before the first event returns None
        let bob = crdt.validator_at_height(&storage, "bob", 1).unwrap();
        assert_eq!(bob, None);

        // All the other state changes are a no op
        crdt.add_validator(&mut storage, "bob", "pk_b", 300, 3456)
            .unwrap();
        crdt.update_validator(&mut storage, "bob", "pk_b", 400, 4567)
            .unwrap();
        crdt.remove_validator(&mut storage, "bob", 500, 5678)
            .unwrap();
        crdt.jail_validator(&mut storage, "bob", 600, 6789).unwrap();
        crdt.tombstone_validator(&mut storage, "bob", 800, 8901)
            .unwrap();

        // Querying after the last event returns the initial tombstone
        let bob = crdt.validator_at_height(&storage, "bob", 900).unwrap();
        assert_eq!(
            bob,
            Some(ValState {
                pub_key: "".to_string(),
                start_height: 100,
                start_time: 1234,
                state: State::Tombstoned {}
            })
        );
    }

    #[test]
    fn tombstone_drains_later_events() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "pk_a", 100, 1234)
            .unwrap();
        crdt.add_validator(&mut storage, "bob", "pk_b", 200, 2345)
            .unwrap();
        crdt.tombstone_validator(&mut storage, "bob", 199, 2344)
            .unwrap();

        assert!(!crdt.is_active_validator(&storage, "bob").unwrap());

        let active = crdt.list_active_validators(&storage, None, 10).unwrap();
        assert_eq!(active, vec!["alice".to_string()]);

        // Querying bob returns the tombstone
        let bob = crdt.validator_at_height(&storage, "bob", 500).unwrap();
        assert_eq!(
            bob,
            Some(ValState {
                pub_key: "".to_string(),
                start_height: 199,
                start_time: 2344,
                state: State::Tombstoned {}
            })
        );
    }

    #[test]
    fn drain_older_works() {
        let unbonding_period = 100;
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", "pk_a", 100, 1234)
            .unwrap();
        assert!(crdt.is_active_validator(&storage, "alice").unwrap());

        crdt.remove_validator(&mut storage, "alice", 200, 2345)
            .unwrap();
        assert!(!crdt.is_active_validator(&storage, "alice").unwrap());
        crdt.add_validator(&mut storage, "alice", "pk_b", 300, 3456)
            .unwrap();
        assert!(crdt.is_active_validator(&storage, "alice").unwrap());

        let alice_history = crdt
            .validators
            .may_load(&storage, "alice".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(alice_history.0.len(), 3);

        // Try to drain older events too soon
        let current_time = 1300;
        crdt.drain_older(&mut storage, "alice", current_time - unbonding_period)
            .unwrap();

        // Nothing happens
        let alice_history = crdt
            .validators
            .may_load(&storage, "alice".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(alice_history.0.len(), 3);

        // Try to drain older events a little later
        let current_time = 3500;
        crdt.drain_older(&mut storage, "alice", current_time - unbonding_period)
            .unwrap();

        // Older events are drained
        let alice_history = crdt
            .validators
            .may_load(&storage, "alice".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(alice_history.0.len(), 1);
        // State didn't change
        assert!(crdt.is_active_validator(&storage, "alice").unwrap());

        // Try to drain older events much later
        let current_time = 50000;
        crdt.drain_older(&mut storage, "alice", current_time - unbonding_period)
            .unwrap();

        // Nothing happens (one event is always kept)
        let alice_history = crdt
            .validators
            .may_load(&storage, "alice".to_string())
            .unwrap()
            .unwrap();
        assert_eq!(alice_history.0.len(), 1);
        // State didn't change
        assert!(crdt.is_active_validator(&storage, "alice").unwrap());
    }
}
