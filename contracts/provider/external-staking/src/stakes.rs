use crate::state::Stake;
use cosmwasm_std::{Addr, Order, StdResult, Storage};
use cw_storage_plus::{Index, IndexList, IndexedMap, KeyDeserialize, MultiIndex};

pub struct StakeIndexes<'a> {
    // Last type param defines the pk deserialization type
    pub rev: MultiIndex<'a, (String, Addr), Stake, (Addr, String)>,
}

impl<'a> IndexList<Stake> for StakeIndexes<'a> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Stake>> + '_> {
        let v: Vec<&dyn Index<Stake>> = vec![&self.rev];
        Box::new(v.into_iter())
    }
}

pub struct Stakes<'a> {
    pub stake: IndexedMap<'a, (&'a Addr, &'a str), Stake, StakeIndexes<'a>>,
}

impl<'a> Stakes<'a> {
    fn deserialize_pk(pk: &[u8]) -> (Addr, String) {
        <(Addr, String)>::from_slice(pk).unwrap() // mustn't fail
    }

    pub fn new(storage_key: &'a str, validator_subkey: &'a str) -> Self {
        let indexes = StakeIndexes {
            rev: MultiIndex::new(
                |pk, _| {
                    let (user, validator) = Self::deserialize_pk(pk);
                    (validator, user)
                },
                storage_key,
                validator_subkey,
            ),
        };
        let stakes = IndexedMap::new(storage_key, indexes);

        Self { stake: stakes }
    }

    pub fn stakes_by_validator(
        &self,
        storage: &dyn Storage,
        validator: &str,
    ) -> StdResult<Vec<(Addr, Stake)>> {
        self.stake
            .idx
            .rev
            .sub_prefix(validator.to_string())
            .range(storage, None, None, Order::Ascending)
            .map(|item| {
                let ((user, _), stake) = item?;
                Ok((user, stake))
            })
            .collect::<StdResult<Vec<(Addr, Stake)>>>()
    }
}
