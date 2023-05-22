use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Index, IndexList, UniqueIndex};

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking (only native tokens)
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub local_staking: Addr,
}

/// Single Lien description
#[cw_serde]
pub struct Lien {
    /// Lien creditor, unique across liens
    creditor: Addr,
    /// Credit amount (denom is in `Config::denom`)
    amount: Uint128,
    /// Slashable part - restricted to [0; 1] range
    slashable: Decimal,
}

/// Index type for liens indexed map
pub struct LiensIndex<'a> {
    creditor: UniqueIndex<'a, Addr, Lien, Addr>,
}

impl LiensIndex<'_> {
    pub fn new() -> Self {
        let creditor = UniqueIndex::new(|lien: &Lien| lien.creditor.clone(), "liens__creditor");

        Self { creditor }
    }
}

impl IndexList<Lien> for LiensIndex<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Lien>> + '_> {
        Box::new(std::iter::once(&self.creditor as &_))
    }
}

/// All values are in Config.denom
#[cw_serde]
pub struct Balance {
    pub bonded: Uint128,
    pub claims: Vec<LienAddr>,
}

#[cw_serde]
pub struct LienAddr {
    pub lienholder: Addr,
    pub amount: Uint128,
}
