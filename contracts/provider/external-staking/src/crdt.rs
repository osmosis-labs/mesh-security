use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Order, StdError, StdResult, Storage};
use cw_storage_plus::{Bound, Map};

// Question: Do we need to add more info here if we want to keep historical info for slashing.
// Would we ever need the pubkeys for a Tombstoned validator? Or do we consider it already slashed and therefore unslashable?
#[cw_serde]
pub enum ValidatorState {
    Active(ActiveState),
    Tombstoned {},
}

impl ValidatorState {
    pub fn is_active(&self) -> bool {
        matches!(self, ValidatorState::Active(_))
    }
}

#[cw_serde]
/// Active state maintains a sorted list of updates with no duplicates.
/// The first one is the one with the highest start_height.
pub struct ActiveState(Vec<ValUpdate>);

impl ActiveState {
    /// Add one more element to this list, maintaining the constraints
    pub fn insert_unique(&mut self, update: ValUpdate) {
        self.0.push(update);
        self.0.sort_by(|a, b| b.start_height.cmp(&a.start_height));
        self.0.dedup();
    }
}

#[cw_serde]
pub struct ValUpdate {
    pub pub_key: String,
    pub start_height: u64,
    pub start_time: u64,
}

impl ValUpdate {
    pub fn new(pub_key: impl Into<String>, start_height: u64, start_time: u64) -> Self {
        ValUpdate {
            pub_key: pub_key.into(),
            start_height,
            start_time,
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

    pub fn add_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
        update: ValUpdate,
    ) -> Result<(), StdError> {
        let mut state = self
            .validators
            .may_load(storage, valoper)?
            .unwrap_or_else(|| ValidatorState::Active(ActiveState(vec![])));

        match &mut state {
            ValidatorState::Active(active) => {
                // add to the set, ensuring there are no duplicates
                active.insert_unique(update);
            }
            ValidatorState::Tombstoned {} => {
                // we just silently ignore it here
            }
        }

        self.validators.save(storage, valoper, &state)
    }

    pub fn remove_validator(
        &self,
        storage: &mut dyn Storage,
        valoper: &str,
    ) -> Result<(), StdError> {
        let state = ValidatorState::Tombstoned {};
        self.validators.save(storage, valoper, &state)
    }

    pub fn is_active_validator(&self, storage: &dyn Storage, valoper: &str) -> StdResult<bool> {
        let active = self
            .validators
            .may_load(storage, valoper)?
            .map(|s| s.is_active())
            .unwrap_or(false);
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
                Ok((valoper, ValidatorState::Active(_))) => Some(Ok(valoper)),
                Ok((_, ValidatorState::Tombstoned {})) => None,
                Err(e) => Some(Err(e)),
            })
            .take(limit)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use cosmwasm_std::MemoryStorage;

    fn mock_update(start_height: u64) -> ValUpdate {
        ValUpdate {
            pub_key: "TODO".to_string(),
            start_height,
            start_time: 1687339542,
        }
    }

    // We add three new validators, and remove one
    #[test]
    fn happy_path() {
        let mut storage = MemoryStorage::new();
        let crdt = CrdtState::new();

        crdt.add_validator(&mut storage, "alice", mock_update(123))
            .unwrap();
        crdt.add_validator(&mut storage, "bob", mock_update(200))
            .unwrap();
        crdt.add_validator(&mut storage, "carl", mock_update(303))
            .unwrap();
        crdt.remove_validator(&mut storage, "bob").unwrap();

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

        crdt.remove_validator(&mut storage, "bob").unwrap();
        crdt.add_validator(&mut storage, "alice", mock_update(123))
            .unwrap();
        crdt.add_validator(&mut storage, "bob", mock_update(200))
            .unwrap();
        crdt.add_validator(&mut storage, "carl", mock_update(303))
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
            crdt.add_validator(&mut storage, v, mock_update(123))
                .unwrap();
        }
        // in reverse order, so remove doesn't shift the indexes we will later read
        for i in [19, 17, 12, 11, 7, 4, 3] {
            crdt.remove_validator(&mut storage, &validators[i]).unwrap();
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

    // TODO: test key rotation later
}
