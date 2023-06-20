use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_NAME: &str = "mesh-security";
pub const ORDERING: cosmwasm_std::IbcOrder = cosmwasm_std::IbcOrder::Unordered;

#[derive(thiserror::Error, Debug)]
pub enum VersionError {
    #[error("Parse: {0}")]
    Std(#[from] cosmwasm_std::StdError),
    #[error("Invalid protocol name: {0}")]
    InvalidProtocol(String),
    #[error("Invalid version: {0}")]
    InvalidVersion(String),
    #[error("Proposed version {proposed} older than min supported {supported}")]
    VersionTooOld { proposed: String, supported: String },
    #[error("Proposed version {proposed} has breaking changes ahead of supported {supported}")]
    VersionTooNew { proposed: String, supported: String },
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
        Version::parse(&self.version)
            .map_err(|_| VersionError::InvalidVersion(self.version.clone()))
    }

    /// Call this to do the version handshake negotiation. This includes validation
    /// If it is below the min supported version, return an error.
    /// If it is has a higher major version than the supported version, return an error.
    /// Otherwise return min(self.version, supported_version)
    pub fn build_response(
        &self,
        supported_ver: Version,
        min_ver: Version,
    ) -> Result<ProtocolVersion, VersionError> {
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
}
