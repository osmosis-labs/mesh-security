use cosmwasm_std::{Addr, Order, StdResult, Storage};
use cw_storage_plus::{Index, IndexList, IndexedMap, MultiIndex};
use mesh_sync::Tx;
use mesh_sync::Tx::InFlightStaking;

pub struct TxIndexes<'a> {
    // Last type param defines the pk deserialization type
    pub users: MultiIndex<'a, Addr, Tx, Addr>,
}

impl<'a> IndexList<Tx> for TxIndexes<'a> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Tx>> + '_> {
        let v: Vec<&dyn Index<Tx>> = vec![&self.users];
        Box::new(v.into_iter())
    }
}

pub struct Txs {
    pub txs: IndexedMap<u64, Tx, TxIndexes<'static>>,
}

impl Txs {
    pub fn new(storage_key: &'static str, user_subkey: &'static str) -> Self {
        let indexes = TxIndexes {
            users: MultiIndex::new(
                |_, tx| {
                    let user = match tx {
                        InFlightStaking { user, .. } => user,
                        _ => panic!("Invalid tx type"),
                    };
                    user.clone()
                },
                storage_key,
                user_subkey,
            ),
        };
        let txs = IndexedMap::new(storage_key, indexes);

        Self { txs }
    }

    pub fn txs_by_user(&self, storage: &dyn Storage, user: &Addr) -> StdResult<Vec<Tx>> {
        self.txs
            .idx
            .users
            .prefix(user.clone())
            .range(storage, None, None, Order::Ascending)
            .map(|item| {
                let (_, tx) = item?;
                Ok(tx)
            })
            .collect::<StdResult<Vec<Tx>>>()
    }
}
