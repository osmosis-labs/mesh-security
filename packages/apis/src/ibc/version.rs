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
    pub fn new(protocol: &str, version: &str) -> Self {
        ProtocolVersion {
            protocol: protocol.to_string(),
            version: version.to_string(),
        }
    }

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
                supported: min_ver.to_string(),
            })
        } else if proposed.major > supported_ver.major {
            Err(VersionError::VersionTooNew {
                proposed: proposed.to_string(),
                supported: supported_ver.to_string(),
            })
        } else {
            let ver = std::cmp::min(proposed, supported_ver);
            Ok(ProtocolVersion {
                protocol: PROTOCOL_NAME.to_string(),
                version: ver.to_string(),
            })
        }
    }

    /// This is like build_response, but called in the Ack/Confirm step.
    /// The only difference is if the version is higher than the supported_ver,
    /// but has the same major version, we error, as there is no further
    /// possibility to negotiate.
    pub fn verify_compatibility(
        &self,
        supported_ver: &str,
        min_ver: &str,
    ) -> Result<(), VersionError> {
        let supported_ver = parse_version(supported_ver)?;
        let min_ver = parse_version(min_ver)?;
        let proposed = self.validate()?;
        if proposed < min_ver {
            Err(VersionError::VersionTooOld {
                proposed: proposed.to_string(),
                supported: min_ver.to_string(),
            })
        } else if proposed > supported_ver {
            // we compare full version, not just major version like above
            Err(VersionError::VersionTooNew {
                proposed: proposed.to_string(),
                supported: supported_ver.to_string(),
            })
        } else {
            Ok(())
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
    use super::*;

    #[test]
    fn validate_works() {
        let valid = ProtocolVersion::new(PROTOCOL_NAME, "1.2.3");
        assert_eq!(valid.validate().unwrap(), Version::new(1, 2, 3));
        // we can only verify it validates. I cannot see how to manually construct the alpha.1 ending without parse or crate-private functions
        let valid_alpha = ProtocolVersion::new(PROTOCOL_NAME, "1.3.2-alpha.1");
        valid_alpha.validate().unwrap();

        // invalid version numbering
        let err = ProtocolVersion::new(PROTOCOL_NAME, "1.2.3a")
            .validate()
            .unwrap_err();
        assert_eq!(err, VersionError::InvalidVersion("1.2.3a".to_string()));

        // invalid protocol name
        let err = ProtocolVersion::new("mashup-security", "1.2.3")
            .validate()
            .unwrap_err();
        assert_eq!(
            err,
            VersionError::InvalidProtocol("mashup-security".to_string())
        );
    }

    #[test]
    fn to_string_works() {
        let supported = ProtocolVersion::new(PROTOCOL_NAME, "1.2.3");
        assert_eq!(
            supported.to_string().unwrap(),
            r#"{"protocol":"mesh-security","version":"1.2.3"}"#.to_string()
        );
    }

    #[test]
    fn build_response_works() {
        // they propose the same version we want
        let supported = ProtocolVersion::new(PROTOCOL_NAME, "1.2.3");
        let response = supported.build_response("1.2.3", "1.0.2").unwrap();
        assert_eq!(response, supported);

        // they propose a newer version, but same major version
        let a_bit_newer = ProtocolVersion::new(PROTOCOL_NAME, "1.3.0");
        let response = a_bit_newer.build_response("1.2.3", "1.0.2").unwrap();
        assert_eq!(response, supported);

        // they propose older but supported version
        let a_bit_older = ProtocolVersion::new(PROTOCOL_NAME, "1.0.7");
        let response = a_bit_older.build_response("1.2.3", "1.0.2").unwrap();
        assert_eq!(response, a_bit_older);

        // they propose something with higher major version
        let too_new = ProtocolVersion::new(PROTOCOL_NAME, "2.0.0-alpha.1");
        let err = too_new.build_response("1.2.3", "1.0.2").unwrap_err();
        assert_eq!(
            err,
            VersionError::VersionTooNew {
                proposed: "2.0.0-alpha.1".to_string(),
                supported: "1.2.3".to_string()
            }
        );

        // they propose before our min supported version
        let too_old = ProtocolVersion::new(PROTOCOL_NAME, "1.0.0");
        let err = too_old.build_response("1.2.3", "1.0.2").unwrap_err();
        assert_eq!(
            err,
            VersionError::VersionTooOld {
                proposed: "1.0.0".to_string(),
                supported: "1.0.2".to_string()
            }
        );
    }

    #[test]
    fn verify_compatibility() {
        // they propose the same version we want - GOOD!
        let supported = ProtocolVersion::new(PROTOCOL_NAME, "1.2.3");
        supported.verify_compatibility("1.2.3", "1.0.2").unwrap();

        // they propose a newer version, but same major version - BAD!
        let a_bit_newer = ProtocolVersion::new(PROTOCOL_NAME, "1.3.0");
        let err = a_bit_newer
            .verify_compatibility("1.2.3", "1.0.2")
            .unwrap_err();
        assert_eq!(
            err,
            VersionError::VersionTooNew {
                proposed: "1.3.0".to_string(),
                supported: "1.2.3".to_string()
            }
        );

        // they propose older but supported version - GOOD
        let a_bit_older = ProtocolVersion::new(PROTOCOL_NAME, "1.0.7");
        a_bit_older.verify_compatibility("1.2.3", "1.0.2").unwrap();

        // they propose something with higher major version - BAD
        let too_new = ProtocolVersion::new(PROTOCOL_NAME, "2.0.0-alpha.1");
        let err = too_new.verify_compatibility("1.2.3", "1.0.2").unwrap_err();
        assert_eq!(
            err,
            VersionError::VersionTooNew {
                proposed: "2.0.0-alpha.1".to_string(),
                supported: "1.2.3".to_string()
            }
        );

        // they propose before our min supported version - BAD!
        let too_old = ProtocolVersion::new(PROTOCOL_NAME, "1.0.0");
        let err = too_old.verify_compatibility("1.2.3", "1.0.2").unwrap_err();
        assert_eq!(
            err,
            VersionError::VersionTooOld {
                proposed: "1.0.0".to_string(),
                supported: "1.0.2".to_string()
            }
        );
    }
}
