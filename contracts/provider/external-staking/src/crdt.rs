use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Order, StdError, StdResult, Storage};
use cw_storage_plus::{Bound, Map};

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

    pub fn is_active(&self) -> bool {
        !self.is_empty() && self.0[0].state == State::Active {}
    }

    pub fn is_tombstoned(&self) -> bool {
        !self.is_empty() && self.0[0].state == State::Tombstoned {}
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
pub enum State {
    /// Validator is part of the validator set.
    Active {},
    /// Validator is not part of the validator set due to being unbonded.
    Unbonded {},
    /// Validator is not part of the validator set due to being jailed.
    Jailed {},
    /// Validator is not part of the validator set due to being tombstoned.
    Tombstoned {},
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
pub struct CrdtState<'a> {
    validators: Map<'a, &'a str, ValidatorState>,
}

impl<'a> CrdtState<'a> {
    pub const fn new() -> Self {
        CrdtState {
            validators: Map::new("crdt.validators"),
        }
    }

    /// Add / Update a validator.
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
            .may_load(storage, valoper)?
            .unwrap_or_else(|| ValidatorState(vec![]));

        // We just silently ignore it if already tombstoned
        if !validator_state.is_tombstoned() {
            let val_state = ValState {
                pub_key: pub_key.to_string(),
                start_height: height,
                start_time: time,
                state: State::Active {},
            };
            validator_state.insert_unique(val_state);
            // TODO: drain events that are older than unbonding period (maintenance)
            self.validators.save(storage, valoper, &validator_state)?;
        }
        Ok(())
    }

    /// Remove a validator.
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
            .may_load(storage, valoper)?
            .unwrap_or_else(|| ValidatorState(vec![]));

        // We just silently ignore it if already tombstoned
        if !validator_state.is_tombstoned() {
            let val_state = ValState {
                pub_key: "TODO".to_string(),
                start_height: height,
                start_time: time,
                state: State::Tombstoned {},
            };
            validator_state.insert_unique(val_state);
            // TODO: drain events that are newer than `height` (this is the final registered event)
            // TODO: drain events that are older than unbonding period (maintenance)
            self.validators.save(storage, valoper, &validator_state)?;
        }
        Ok(())
    }

    pub fn is_active_validator(&self, storage: &dyn Storage, valoper: &str) -> StdResult<bool> {
        let active = self
            .validators
            .may_load(storage, valoper)?
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

    pub fn active_validator(
        &self,
        storage: &dyn Storage,
        valoper: &str,
    ) -> StdResult<Option<ValState>> {
        let state = self.validators.load(storage, valoper)?;
        if state.is_active() {
            Ok(state.0.first().cloned())
        } else {
            Ok(None)
        }
    }

    pub fn active_validator_at_height(
        &self,
        storage: &dyn Storage,
        valoper: &str,
        height: u64,
    ) -> StdResult<Option<ValState>> {
        let state = self.validators.load(storage, valoper)?;
        match state.query_at_height(height) {
            Some(val_state) if val_state.state == State::Active {} => Ok(Some(val_state.clone())),
            Some(_) => Ok(None),
            None => Ok(None),
        }
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
        crdt.remove_validator(&mut storage, "bob", 201, 2346)
            .unwrap();

        assert!(crdt.is_active_validator(&storage, "alice").unwrap());
        assert!(!crdt.is_active_validator(&storage, "bob").unwrap());
        assert!(crdt.is_active_validator(&storage, "carl").unwrap());

        let active = crdt.list_active_validators(&storage, None, 10).unwrap();
        assert_eq!(active, vec!["alice".to_string(), "carl".to_string()]);
    }

    // Like happy path, but we remove bob before he was ever added
    #[test]
    fn remove_before_add_works() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.remove_validator(&mut storage, "bob", 199, 2344)
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
            crdt.remove_validator(&mut storage, &validators[i], 200, 2345)
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
        crdt.add_validator(&mut storage, "alice", "alice_pubkey_2", 202, 2347)
            .unwrap();
        crdt.add_validator(&mut storage, "alice", "alice_pubkey_3", 203, 2348)
            .unwrap();

        // query before update
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

        // query at 2nd update height
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

        // query after last update height
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
}
