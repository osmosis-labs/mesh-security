use cosmwasm_schema::cw_serde;

use crate::error::ContractError;

#[cw_serde]
pub struct AuthorizedEndpoint {
    pub connection_id: String,
    pub port_id: String,
}

impl AuthorizedEndpoint {
    pub fn new(connection_id: &str, port_id: &str) -> Self {
        Self {
            connection_id: connection_id.into(),
            port_id: port_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        // FIXME: can we add more checks here? is this formally defined in some ibc spec?
        if self.connection_id.is_empty() || self.port_id.is_empty() {
            return Err(ContractError::InvalidEndpoint(format!("{:?}", self)));
        }
        Ok(())
    }
}
