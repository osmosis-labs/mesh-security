use cosmwasm_std::{Decimal, Uint128};

pub struct UsedCollateral {
    // Highes user lien
    pub max_lien: Uint128,
    // Tatal slashable amount for user
    pub total_slashable: Uint128,
}

impl UsedCollateral {
    // Return total used collateral
    pub fn used(&self) -> Uint128 {
        self.max_lien.max(self.total_slashable)
    }

    // Adds new lien
    pub fn add_lien(&mut self, amount: Uint128, slashable: Decimal) {
        self.max_lien = self.max_lien.max(amount);
        self.total_slashable += amount * slashable;
    }
}
