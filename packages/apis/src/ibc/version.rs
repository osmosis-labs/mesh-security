use cosmwasm_std::{to_vec, IbcOrder, StdResult};
use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_NAME: &str = "mesh-security";
pub const ORDERING: cosmwasm_std::IbcOrder = cosmwasm_std::IbcOrder::Unordered;

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum VersionError {
    #[error("Invalid protocol name: {0}")]
    InvalidProtocol(String),
    #[error("Invalid version: {0}")]
    InvalidVersion(String),
    #[error("Proposed version {proposed} older than min supported {supported}")]
    VersionTooOld { proposed: String, supported: String },
    #[error("Proposed version {proposed} has breaking changes ahead of supported {supported}")]
    VersionTooNew { proposed: String, supported: String },
    #[error("Channel must be unordered")]
    InvalidChannelOrder,
}

fn parse_version(version: &str) -> Result<Version, VersionError> {
    Version::parse(version).map_err(|_| VersionError::InvalidVersion(version.to_string()))
}

/// Implements logic as defined here:
/// https://github.com/osmosis-labs/mesh-security/blob/main/docs/ibc/ControlChannel.md#establishing-a-channel
/// (Note the comment not to use cw_serde)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ProtocolVersion {
    pub protocol: String,
    pub version: String,
}

impl ProtocolVersion {
    pub fn validate(&self) -> Result<Version, VersionError> {
        if self.protocol != PROTOCOL_NAME {
            return Err(VersionError::InvalidProtocol(self.protocol.clone()));
        }
        parse_version(&self.version)
    }

    /// Call this to do the version handshake negotiation. This includes validation
    /// If it is below the min supported version, return an error.
    /// If it is has a higher major version than the supported version, return an error.
    /// Otherwise return min(self.version, supported_version)
    pub fn build_response(
        &self,
        supported_ver: &str,
        min_ver: &str,
    ) -> Result<ProtocolVersion, VersionError> {
        let supported_ver = parse_version(supported_ver)?;
        let min_ver = parse_version(min_ver)?;
        let proposed = self.validate()?;
        if proposed < min_ver {
            Err(VersionError::VersionTooOld {
                proposed: proposed.to_string(),
                supported: self.version.to_string(),
            })
        } else if proposed.major > supported_ver.major {
            Err(VersionError::VersionTooNew {
                proposed: proposed.to_string(),
                supported: self.version.to_string(),
            })
        } else {
            let ver = std::cmp::min(proposed, supported_ver);
            Ok(ProtocolVersion {
                protocol: PROTOCOL_NAME.to_string(),
                version: ver.to_string(),
            })
        }
    }

    pub fn to_string(&self) -> StdResult<String> {
        let bytes = to_vec(self)?;
        Ok(String::from_utf8(bytes)?)
    }
}

pub fn validate_channel_order(check: &IbcOrder) -> Result<(), VersionError> {
    if check == &ORDERING {
        Ok(())
    } else {
        Err(VersionError::InvalidChannelOrder)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn todo_implement_tests() {
        todo!();
    }
}
