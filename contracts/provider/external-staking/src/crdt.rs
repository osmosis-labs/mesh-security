use cosmwasm_schema::cw_serde;
use cosmwasm_std::{StdError, Storage};
use cw_storage_plus::Map;

#[cw_serde]
pub enum ValidatorState {
    Active(ActiveState),
    Tombstoned {},
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
}
