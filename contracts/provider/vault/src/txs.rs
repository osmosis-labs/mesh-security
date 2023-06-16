use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Order, StdResult, Storage, Uint128};
use cw_storage_plus::{Index, IndexList, IndexedMap, MultiIndex};

#[cw_serde]
pub enum TxType {
    InFlightStaking,
    // TODO
    // Slash,
}

#[cw_serde]
pub struct Tx {
    /// Transaction id
    pub id: u64,
    /// Transaction type
    pub ty: TxType,
    /// Associated amount
    pub amount: Uint128,
    /// Slashable portion of lien
    pub slashable: Decimal,
    /// Associated user
    pub user: Addr,
    /// Remote staking contract
    pub lienholder: Addr,
}

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

pub struct Txs<'a> {
    pub txs: IndexedMap<'a, u64, Tx, TxIndexes<'a>>,
}

impl<'a> Txs<'a> {
    pub fn new(storage_key: &'a str, user_subkey: &'a str) -> Self {
        let indexes = TxIndexes {
            users: MultiIndex::new(|_, tx| tx.user.clone(), storage_key, user_subkey),
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
